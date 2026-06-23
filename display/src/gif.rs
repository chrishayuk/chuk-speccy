//! Encode indexed framebuffers as an animated GIF — the lightweight, ffmpeg-free
//! sibling of the host MP4 path. Reusable by any head or tool that wants a
//! shareable clip: demos, MCP session replays, agent episode captures.

use std::borrow::Cow;
use std::io::{self, Write};

fn to_io(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Encode `frames` (each `width*height` palette indices) to an animated GIF on
/// `out`. `palette` is up to 256 RGB triples — a power-of-two count (the Spectrum
/// uses 16, [`crate::AUTHENTIC`]). `delay_cs` is per-frame delay in centiseconds
/// (e.g. a 50 Hz frame decimated by 2 ≈ 4 cs). Loops forever.
pub fn encode_indexed<W: Write>(
    out: W,
    frames: &[Vec<u8>],
    width: u16,
    height: u16,
    palette: &[[u8; 3]],
    delay_cs: u16,
) -> io::Result<()> {
    let flat: Vec<u8> = palette.iter().flatten().copied().collect();
    let mut enc = gif::Encoder::new(out, width, height, &flat).map_err(to_io)?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(to_io)?;
    for f in frames {
        let mut frame = gif::Frame {
            width,
            height,
            delay: delay_cs,
            buffer: Cow::Borrowed(f.as_slice()),
            ..Default::default()
        };
        // Re-send the (small) palette per frame? No — the global palette covers it.
        frame.palette = None;
        enc.write_frame(&frame).map_err(to_io)?;
    }
    Ok(())
}

/// Convenience: encode to a `Vec<u8>`.
pub fn encode_indexed_to_vec(
    frames: &[Vec<u8>],
    width: u16,
    height: u16,
    palette: &[[u8; 3]],
    delay_cs: u16,
) -> Vec<u8> {
    let mut buf = Vec::new();
    encode_indexed(&mut buf, frames, width, height, palette, delay_cs)
        .expect("Vec write is infallible");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_valid_gif() {
        // 2x2, two frames, 2-colour palette.
        let pal = [[0, 0, 0], [255, 255, 255]];
        let frames = vec![vec![0, 1, 1, 0], vec![1, 0, 0, 1]];
        let bytes = encode_indexed_to_vec(&frames, 2, 2, &pal, 5);
        assert_eq!(&bytes[..6], b"GIF89a", "valid GIF header");
        assert!(bytes.len() > 20, "has content");
    }
}
