//! What an animated picture is (canvas and frame count), read from its headers without decoding it,
//! so the app can tell an animation from a still and refuse a canvas too big to play.

/// Canvas width, height and frame count of a GIF, animated WebP or APNG with more than one frame;
/// `None` for a still picture or any other format.
pub fn canvas(b: &[u8]) -> Option<(usize, usize, usize)> {
    let (w, h, frames) = if b.starts_with(b"GIF8") { gif(b)? }
        else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" { webp(b)? }
        else if b.starts_with(b"\x89PNG\r\n\x1a\n") { apng(b)? }
        else { return None };
    (frames > 1).then_some((w, h, frames))
}

/// What Rune's cache would take decoding all of it: 16/3 bytes a pixel a frame (kept only to say
/// how much the streaming player saves).
#[cfg_attr(not(test), allow(dead_code))]
pub fn decoded_bytes(b: &[u8]) -> Option<usize> { canvas(b).map(|(w, h, frames)| w * h * frames * 16 / 3) }

fn skip_sub_blocks(b: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let n = *b.get(pos)? as usize;
        pos += 1 + n;
        if n == 0 { return Some(pos) }
    }
}

fn gif(b: &[u8]) -> Option<(usize, usize, usize)> {
    if b.len() < 13 { return None }
    let (w, h) = (u16::from_le_bytes([b[6], b[7]]) as usize, u16::from_le_bytes([b[8], b[9]]) as usize);
    let table = |flags: u8| if flags & 0x80 != 0 { 3 * (1usize << ((flags & 7) + 1)) } else { 0 };
    let mut pos = 13 + table(b[10]);
    let mut frames = 0;
    // A truncated file counts the frames it has, as the decoder plays them.
    while let Some(&kind) = b.get(pos) {
        match kind {
            0x21 => match skip_sub_blocks(b, pos + 2) { Some(p) => pos = p, None => break },
            0x2C => {
                let Some(&flags) = b.get(pos + 9) else { break };
                frames += 1;
                match skip_sub_blocks(b, pos + 10 + table(flags) + 1) { Some(p) => pos = p, None => break }
            }
            _ => break,
        }
    }
    Some((w, h, frames))
}

fn webp(b: &[u8]) -> Option<(usize, usize, usize)> {
    let (mut pos, mut size, mut frames) = (12, None, 0);
    while pos + 8 <= b.len() {
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let data = pos + 8;
        match &b[pos..pos + 4] {
            b"VP8X" if len >= 10 && data + 10 <= b.len() => {
                let u24 = |i: usize| b[i] as usize | (b[i + 1] as usize) << 8 | (b[i + 2] as usize) << 16;
                size = Some((u24(data + 4) + 1, u24(data + 7) + 1));
            }
            b"ANMF" => frames += 1,
            _ => {}
        }
        pos = data + len + (len & 1);
    }
    size.map(|(w, h)| (w, h, frames))
}

fn apng(b: &[u8]) -> Option<(usize, usize, usize)> {
    let (mut pos, mut size, mut frames) = (8, None, 1);
    while pos + 8 <= b.len() {
        let len = u32::from_be_bytes(b[pos..pos + 4].try_into().ok()?) as usize;
        let data = pos + 8;
        match &b[pos + 4..pos + 8] {
            b"IHDR" if data + 8 <= b.len() => {
                size = Some((u32::from_be_bytes(b[data..data + 4].try_into().ok()?) as usize, u32::from_be_bytes(b[data + 4..data + 8].try_into().ok()?) as usize));
            }
            b"acTL" if data + 4 <= b.len() => frames = u32::from_be_bytes(b[data..data + 4].try_into().ok()?) as usize,
            // The animation control chunk comes before the first picture data.
            b"IDAT" => break,
            _ => {}
        }
        pos = data + len + 4;
    }
    size.map(|(w, h)| (w, h, frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif_of(frames: usize) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend([100, 0, 50, 0, 0x80, 0, 0]);
        b.extend([0u8; 6]); // a 2-colour global table
        for _ in 0..frames {
            b.extend([0x21, 0xF9, 4, 0, 10, 0, 0, 0]); // graphic control extension
            b.extend([0x2C, 0, 0, 0, 0, 100, 0, 50, 0, 0]); // image descriptor, no local table
            b.extend([2, 3, 1, 2, 3, 0]); // LZW size and one data block
        }
        b.push(0x3B);
        b
    }

    #[test]
    fn a_gif_is_sized_by_its_canvas_and_frames_and_a_still_is_not_an_animation() {
        assert_eq!(decoded_bytes(&gif_of(30)), Some(100 * 50 * 30 * 16 / 3));
        assert_eq!(decoded_bytes(&gif_of(1)), None, "one frame is a still");
        assert_eq!(decoded_bytes(b"\xff\xd8\xff not an animation"), None);
        let cut = gif_of(30);
        assert_eq!(decoded_bytes(&cut[..cut.len() / 2]).map(|n| n < 100 * 50 * 30 * 16 / 3), Some(true), "a truncated file counts what it has");
    }

    #[test]
    fn an_animated_webp_is_sized_from_its_canvas_and_frame_chunks() {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend(b"VP8X");
        b.extend(10u32.to_le_bytes());
        b.extend([2, 0, 0, 0]); // flags: animation
        b.extend([239, 0, 0]); // canvas width - 1 = 239
        b.extend([119, 0, 0]); // canvas height - 1 = 119
        for _ in 0..4 { b.extend(b"ANMF"); b.extend(3u32.to_le_bytes()); b.extend([1, 2, 3, 0]) }
        assert_eq!(decoded_bytes(&b), Some(240 * 120 * 4 * 16 / 3));
    }

    #[test]
    fn an_apng_is_sized_from_its_header_and_control_chunk() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend(13u32.to_be_bytes()); b.extend(b"IHDR");
        b.extend(64u32.to_be_bytes()); b.extend(32u32.to_be_bytes()); b.extend([8, 6, 0, 0, 0]); b.extend([0; 4]);
        b.extend(8u32.to_be_bytes()); b.extend(b"acTL"); b.extend(5u32.to_be_bytes()); b.extend(0u32.to_be_bytes()); b.extend([0; 4]);
        b.extend(0u32.to_be_bytes()); b.extend(b"IDAT"); b.extend([0; 4]);
        assert_eq!(decoded_bytes(&b), Some(64 * 32 * 5 * 16 / 3));
    }
}
