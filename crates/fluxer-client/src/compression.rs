//! `zstd-stream` gateway compression: one continuous zstd stream per direction, one flushed chunk
//! per WebSocket binary frame. Mirrors `fluxer_app/rust/libfluxcore/src/zstd_stream.rs` and the
//! parameters of `GatewayCompression.ts` (level 3, 128 KiB encoder window).

use zstd::zstd_safe::zstd_sys::ZSTD_EndDirective;
use zstd::zstd_safe::{CCtx, CParameter, DCtx, DParameter, InBuffer, OutBuffer};

const BUF: usize = 16 * 1024;
const MAX_DECOMPRESSED: usize = 64 * 1024 * 1024;
const ENCODER_WINDOW_LOG: u32 = 17;
const ENCODER_TABLE_LOG: u32 = 15;
const DECODER_WINDOW_LOG_MAX: u32 = 21;
pub const LEVEL: i32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum Error { TooLarge, Decode, Encode, Memory }

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "zstd decompressed payload is too large",
            Self::Decode => "zstd decompression failed",
            Self::Encode => "zstd compression failed",
            Self::Memory => "out of memory",
        })
    }
}
impl std::error::Error for Error {}

fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    if out.len() + bytes.len() > MAX_DECOMPRESSED { return Err(Error::TooLarge) }
    out.extend_from_slice(bytes);
    Ok(())
}

pub struct Encoder(CCtx<'static>);

impl Encoder {
    pub fn new(level: i32) -> Result<Self, Error> {
        let mut c = CCtx::try_create().ok_or(Error::Memory)?;
        for p in [
            CParameter::CompressionLevel(level),
            CParameter::WindowLog(ENCODER_WINDOW_LOG),
            CParameter::HashLog(ENCODER_TABLE_LOG),
            CParameter::ChainLog(ENCODER_TABLE_LOG),
        ] {
            c.set_parameter(p).map_err(|_| Error::Encode)?;
        }
        Ok(Self(c))
    }

    /// Compresses `input` and flushes, so the peer can decode this chunk on its own.
    pub fn compress(&mut self, input: &[u8]) -> Result<Vec<u8>, Error> {
        let mut input = InBuffer::around(input);
        let mut out = Vec::new();
        let mut buf = [0u8; BUF];
        loop {
            let before = input.pos();
            let (remaining, written) = {
                let mut ob = OutBuffer::around(&mut buf[..]);
                let r = self.0.compress_stream2(&mut ob, &mut input, ZSTD_EndDirective::ZSTD_e_flush)
                    .map_err(|_| Error::Encode)?;
                (r, ob.pos())
            };
            append(&mut out, &buf[..written])?;
            if remaining == 0 { return Ok(out) }
            if input.pos() == before && written == 0 { return Err(Error::Encode) }
        }
    }
}

pub struct Decoder(DCtx<'static>);

impl Decoder {
    pub fn new() -> Result<Self, Error> {
        let mut c = DCtx::try_create().ok_or(Error::Memory)?;
        c.init().map_err(|_| Error::Decode)?;
        c.set_parameter(DParameter::WindowLogMax(DECODER_WINDOW_LOG_MAX)).map_err(|_| Error::Decode)?;
        Ok(Self(c))
    }

    /// Decodes the next chunk of the stream. Empty output means the chunk only completed part of
    /// a block: wait for more.
    pub fn decompress(&mut self, input: &[u8]) -> Result<Vec<u8>, Error> {
        if input.is_empty() { return Ok(Vec::new()) }
        let mut input = InBuffer::around(input);
        let mut out = Vec::new();
        let mut buf = [0u8; BUF];
        loop {
            let before = input.pos();
            let (remaining, written) = {
                let mut ob = OutBuffer::around(&mut buf[..]);
                let r = self.0.decompress_stream(&mut ob, &mut input).map_err(|_| Error::Decode)?;
                (r, ob.pos())
            };
            append(&mut out, &buf[..written])?;
            if input.pos() == input.src.len() && written < buf.len() { return Ok(out) }
            if input.pos() == before && written == 0 {
                if remaining > 0 && input.pos() == input.src.len() { return Ok(out) }
                return Err(Error::Decode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_roundtrip_on_one_continuous_stream() {
        let (mut e, mut d) = (Encoder::new(LEVEL).unwrap(), Decoder::new().unwrap());
        for i in 0..50 {
            let msg = format!(r#"{{"op":0,"s":{i},"t":"MESSAGE_CREATE","d":{{"content":"olá {i} {}"}}}}"#, "x".repeat(i * 40));
            let wire = e.compress(msg.as_bytes()).unwrap();
            assert_eq!(d.decompress(&wire).unwrap(), msg.as_bytes());
        }
    }

    #[test]
    fn later_chunks_shrink_thanks_to_shared_history() {
        let mut e = Encoder::new(LEVEL).unwrap();
        let msg = br#"{"op":0,"t":"PRESENCE_UPDATE","d":{"status":"online","guild_id":"1234567890"}}"#;
        let first = e.compress(msg).unwrap().len();
        let later = e.compress(msg).unwrap().len();
        assert!(later < first, "{later} < {first}");
    }

    #[test]
    fn garbage_is_an_error() {
        assert_eq!(Decoder::new().unwrap().decompress(&[0xde, 0xad, 0xbe, 0xef, 1, 2, 3]), Err(Error::Decode));
    }
}
