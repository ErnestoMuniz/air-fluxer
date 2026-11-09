//! Fluxer gateway wire layer (transport-agnostic). Mirrors
//! `packages/constants/src/GatewayConstants.ts` and the payload framing in
//! `fluxer_app/src/features/gateway/transport/GatewaySocket.ts`.


pub mod altcha;
pub mod api;
pub mod compression;
pub mod connection;
pub mod rest;
pub mod session;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Opcode(pub u8);

impl Opcode {
    pub const DISPATCH: Self = Self(0);
    pub const HEARTBEAT: Self = Self(1);
    pub const IDENTIFY: Self = Self(2);
    pub const PRESENCE_UPDATE: Self = Self(3);
    pub const VOICE_STATE_UPDATE: Self = Self(4);
    pub const RESUME: Self = Self(6);
    pub const RECONNECT: Self = Self(7);
    pub const INVALID_SESSION: Self = Self(9);
    pub const HELLO: Self = Self(10);
    pub const HEARTBEAT_ACK: Self = Self(11);
    pub const GATEWAY_ERROR: Self = Self(12);
    pub const LAZY_REQUEST: Self = Self(14);
}

/// Identify flags (`GatewayIdentifyFlags`).
pub mod identify_flags {
    pub const DEBOUNCE_MESSAGE_REACTIONS: u32 = 1 << 1;
    pub const CHANNEL_THREADS: u32 = 1 << 2;
    pub const GUILD_VISITS: u32 = 1 << 3;
}

/// One gateway frame: `{ op, d?, s?, t? }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub op: Opcode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

impl Payload {
    pub fn heartbeat(seq: Option<u64>) -> Self {
        Self { op: Opcode::HEARTBEAT, d: seq.map(Value::from), s: None, t: None }
    }
    pub fn decode(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("payload serializes")
    }
}

/// Wire compression negotiated on the URL (`compress=`). `zstd-stream` needs a stream codec
/// (reuse `libfluxcore`'s zstd), so a first client can run with `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression { None, ZstdStream }

pub fn gateway_url(base: &str, api_version: u32, compression: Compression) -> String {
    // Like `new URL(base)` in the web client: a bare host gets the "/" path, so the request line is
    // `GET /?v=1…` (without it some servers answer 400).
    let base = base.trim_end_matches('?');
    let (scheme_host, rest) = match base.split_once("://") {
        Some((scheme, tail)) => {
            let end = tail.find(['/', '?']).unwrap_or(tail.len());
            (format!("{scheme}://{}", &tail[..end]), &tail[end..])
        }
        None => (base.to_string(), ""),
    };
    let path = if rest.is_empty() { "/" } else { rest };
    let mut url = format!("{scheme_host}{path}?v={api_version}&encoding=json");
    match compression {
        Compression::None => url.push_str("&compress=none"),
        Compression::ZstdStream => url.push_str("&compress=zstd-stream&stream=1"),
    }
    url
}

/// Close codes after which reconnecting cannot succeed (`GatewayCloseCodes`).
pub fn is_fatal_close(code: u16) -> bool {
    matches!(code, 4003 | 4004 | 4010 | 4011 | 4012)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_decodes_and_heartbeat_roundtrips() {
        let hello = Payload::decode(r#"{"op":10,"d":{"heartbeat_interval":41250}}"#).unwrap();
        assert_eq!(hello.op, Opcode::HELLO);
        assert_eq!(hello.d.unwrap()["heartbeat_interval"], 41250);
        assert_eq!(Payload::heartbeat(Some(7)).encode(), r#"{"op":1,"d":7}"#);
        assert_eq!(Payload::heartbeat(None).encode(), r#"{"op":1}"#);
    }

    #[test]
    fn dispatch_keeps_sequence_and_event_name() {
        let p = Payload::decode(r#"{"op":0,"s":3,"t":"MESSAGE_CREATE","d":{"id":"1"}}"#).unwrap();
        assert_eq!((p.s, p.t.as_deref()), (Some(3), Some("MESSAGE_CREATE")));
    }

    #[test]
    fn url_and_close_codes() {
        assert_eq!(gateway_url("wss://gw.example/", 1, Compression::None), "wss://gw.example/?v=1&encoding=json&compress=none");
        assert_eq!(gateway_url("wss://gw.example", 1, Compression::None), "wss://gw.example/?v=1&encoding=json&compress=none", "bare host gets a path");
        assert_eq!(gateway_url("wss://gw.example:8443/ws", 1, Compression::ZstdStream), "wss://gw.example:8443/ws?v=1&encoding=json&compress=zstd-stream&stream=1");
        assert!(gateway_url("wss://gw.example/", 1, Compression::ZstdStream).ends_with("&compress=zstd-stream&stream=1"));
        assert!(is_fatal_close(4004) && !is_fatal_close(4009));
    }
}
