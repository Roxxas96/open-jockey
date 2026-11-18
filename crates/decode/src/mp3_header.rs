//! An MP3's first frame, read straight from the file: whether it is a Xing/Info frame, which
//! encoder wrote that frame, and how many samples a frame holds.

use std::io::{Read, Seek, SeekFrom};

use crate::InfoEncoder;

/// How far past the ID3v2 tag the first frame is looked for.
const SEARCH_BYTES: u64 = 64 * 1024;

/// The encoder string sits this far past the "Xing"/"Info" tag (LAME's layout), 9 bytes long.
const ENCODER_OFFSET: usize = 120;
const ENCODER_LEN: usize = 9;

/// What the first frame says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FirstFrame {
    /// The frame is a Xing or Info frame.
    pub info_frame: bool,
    /// The encoder string in that frame's tag, when there is one.
    pub info_encoder: Option<InfoEncoder>,
    /// Samples per frame: 1152 for MPEG-1, 576 for MPEG-2 and 2.5.
    pub frame_samples: u32,
}

/// A Layer III frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    mpeg1: bool,
    mono: bool,
    sample_rate: u32,
    len: usize,
}

/// Decodes the four bytes at the start of `b` as a Layer III frame header.
fn header(b: &[u8]) -> Option<Header> {
    let &[b0, b1, b2, b3, ..] = b else {
        return None;
    };
    if b0 != 0xFF || b1 & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b1 >> 3) & 0b11; // 0: MPEG-2.5, 1: reserved, 2: MPEG-2, 3: MPEG-1
    let layer = (b1 >> 1) & 0b11; // 1: Layer III
    if version == 1 || layer != 1 {
        return None;
    }
    let mpeg1 = version == 3;
    let kbps: u32 = match (mpeg1, b2 >> 4) {
        (_, 0 | 15) => return None, // free format, or invalid
        (true, i) => [
            32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ][usize::from(i - 1)],
        (false, i) => {
            [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][usize::from(i - 1)]
        }
    };
    let base: u32 = match (b2 >> 2) & 0b11 {
        0 => 44_100,
        1 => 48_000,
        2 => 32_000,
        _ => return None,
    };
    let sample_rate = match version {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    let padding = u32::from((b2 >> 1) & 1);
    let coefficient = if mpeg1 { 144 } else { 72 };
    let len = coefficient * kbps * 1000 / sample_rate + padding;
    Some(Header {
        mpeg1,
        mono: b3 >> 6 == 0b11,
        sample_rate,
        len: usize::try_from(len).ok()?,
    })
}

/// Reads the first MP3 frame after `r`'s ID3v2 tag (if any). `None` when no frame is found.
pub(crate) fn first_frame<R: Read + Seek>(r: &mut R) -> Option<FirstFrame> {
    let mut id3 = [0u8; 10];
    r.read_exact(&mut id3).ok()?;
    let start = if &id3[..3] == b"ID3" {
        let size = id3[6..10]
            .iter()
            .fold(0u64, |acc, &b| (acc << 7) | u64::from(b & 0x7F));
        let footer = if id3[5] & 0x10 != 0 { 10 } else { 0 };
        10 + size + footer
    } else {
        0
    };
    r.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    r.take(SEARCH_BYTES).read_to_end(&mut bytes).ok()?;
    scan(&bytes)
}

/// Finds the first frame in `bytes`: a valid header whose next frame also syncs (or would
/// start past the bytes there are).
fn scan(bytes: &[u8]) -> Option<FirstFrame> {
    (0..bytes.len()).find_map(|at| {
        let head = header(&bytes[at..])?;
        let next_at = at + head.len;
        if next_at + 4 <= bytes.len() {
            let next = header(&bytes[next_at..])?;
            if next.mpeg1 != head.mpeg1 || next.sample_rate != head.sample_rate {
                return None;
            }
        }
        Some(read_frame(&bytes[at..], head))
    })
}

/// Reads the Xing/Info tag and its encoder string from the frame at the start of `frame`.
fn read_frame(frame: &[u8], head: Header) -> FirstFrame {
    let side_info = match (head.mpeg1, head.mono) {
        (true, false) => 32,
        (true, true) | (false, false) => 17,
        (false, true) => 9,
    };
    let tag_at = 4 + side_info;
    let info_frame = matches!(frame.get(tag_at..tag_at + 4), Some(b"Xing" | b"Info"));
    let encoder_at = tag_at + ENCODER_OFFSET;
    let info_encoder = info_frame
        .then(|| frame.get(encoder_at..encoder_at + ENCODER_LEN))
        .flatten()
        .and_then(encoder);
    FirstFrame {
        info_frame,
        info_encoder,
        frame_samples: if head.mpeg1 { 1152 } else { 576 },
    }
}

/// Names the encoder from its string: "LAME…", "Lavc…", or another printable name.
fn encoder(name: &[u8]) -> Option<InfoEncoder> {
    if name.starts_with(b"LAME") {
        Some(InfoEncoder::Lame)
    } else if name.starts_with(b"Lavc") {
        Some(InfoEncoder::Lavc)
    } else if name.get(..4)?.iter().all(u8::is_ascii_graphic) {
        Some(InfoEncoder::Other)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use super::*;
    use crate::{InfoEncoder, lead_in};

    fn ojcal(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../interop/tests/fixtures/ojcal")
            .join(name)
    }

    #[test]
    fn reads_each_ojcal_header() {
        let read = |name: &str| lead_in(&ojcal(name)).unwrap();

        let l = read("OJ Cal 01 ffmpeg 44k CBR.mp3");
        assert!(l.info_frame);
        assert_eq!(l.info_encoder, Some(InfoEncoder::Lavc));
        assert_eq!(l.frame_samples, 1152);
        assert_eq!(l.delay_frames, 1105);

        for name in ["OJ Cal 03 lame 44k CBR.mp3", "OJ Cal 04 lame 44k VBR.mp3"] {
            let l = read(name);
            assert!(l.info_frame, "{name}");
            assert_eq!(l.info_encoder, Some(InfoEncoder::Lame), "{name}");
            assert_eq!(l.frame_samples, 1152, "{name}");
            assert_eq!(l.delay_frames, 1105, "{name}");
        }

        let l = read("OJ Cal 06 lame 48k CBR.mp3");
        assert!(l.info_frame);
        assert_eq!(l.info_encoder, Some(InfoEncoder::Lame));
        assert_eq!(l.frame_samples, 1152);

        let l = read("OJ Cal 07 lame 22k CBR.mp3");
        assert!(l.info_frame);
        assert_eq!(l.info_encoder, Some(InfoEncoder::Lame));
        assert_eq!(l.frame_samples, 576);

        for name in [
            "OJ Cal 02 ffmpeg 44k no-Info.mp3",
            "OJ Cal 05 lame 44k no-Info.mp3",
        ] {
            let l = read(name);
            assert!(!l.info_frame, "{name}");
            assert_eq!(l.info_encoder, None, "{name}");
            assert_eq!(l.frame_samples, 1152, "{name}");
        }

        assert_eq!(read("OJ Cal 00 reference 44k.wav").frame_samples, 0);
    }

    /// An ID3v2.4 tag of `body` plus `padding` zero bytes, with a footer when `footer` is set.
    fn id3v24(body: &[u8], padding: usize, footer: bool) -> Vec<u8> {
        let size = body.len() + padding;
        let syncsafe = |n: usize| {
            [
                ((n >> 21) & 0x7F) as u8,
                ((n >> 14) & 0x7F) as u8,
                ((n >> 7) & 0x7F) as u8,
                (n & 0x7F) as u8,
            ]
        };
        let flags = if footer { 0x10 } else { 0 };
        let mut tag = b"ID3\x04\x00".to_vec();
        tag.push(flags);
        tag.extend_from_slice(&syncsafe(size));
        tag.extend_from_slice(body);
        tag.resize(tag.len() + padding, 0);
        if footer {
            tag.extend_from_slice(b"3DI\x04\x00");
            tag.push(flags);
            tag.extend_from_slice(&syncsafe(size));
        }
        tag
    }

    /// A text frame whose body holds a fake MPEG-1 Layer III frame sync.
    fn frame_with_fake_sync() -> Vec<u8> {
        let body = b"\x03\xFF\xFB\x90\x00fake";
        let mut f = b"TIT2".to_vec();
        f.extend_from_slice(&[0, 0, 0, body.len() as u8, 0, 0]);
        f.extend_from_slice(body);
        f
    }

    /// An MPEG-1 Layer III, 128 kb/s, 44.1 kHz joint-stereo frame (417 bytes) whose side
    /// info is followed by `tag` and, 120 bytes on, by `encoder`.
    fn info_frame(tag: &[u8; 4], encoder: &[u8]) -> Vec<u8> {
        let mut f = vec![0xFF, 0xFB, 0x90, 0x40];
        f.resize(4 + 32, 0);
        f.extend_from_slice(tag);
        f.extend_from_slice(&[0, 0, 0, 0x0F]);
        f.resize(4 + 32 + 120, 0);
        f.extend_from_slice(encoder);
        f.resize(417, 0);
        f
    }

    #[test]
    fn junk_and_padding_before_the_first_frame() {
        let mut bytes = id3v24(&frame_with_fake_sync(), 300, true);
        // Junk after the tag, with a sync whose "next frame" lands inside the real one.
        bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00, 0, 0, 0]);
        bytes.extend_from_slice(&info_frame(b"Info", b"LAME3.100"));
        bytes.extend_from_slice(&info_frame(b"\0\0\0\0", b""));

        let f = first_frame(&mut Cursor::new(bytes)).unwrap();
        assert!(f.info_frame);
        assert_eq!(f.info_encoder, Some(InfoEncoder::Lame));
        assert_eq!(f.frame_samples, 1152);
    }

    #[test]
    fn a_cut_info_frame_reads_no_encoder() {
        let mut bytes = id3v24(b"", 16, false);
        let frame = info_frame(b"Info", b"LAME3.100");
        let info_at = 4 + 32;
        bytes.extend_from_slice(&frame[..info_at + 4 + 10]);

        let f = first_frame(&mut Cursor::new(bytes)).unwrap();
        assert!(f.info_frame);
        assert_eq!(f.info_encoder, None);
        assert_eq!(f.frame_samples, 1152);
    }

    #[test]
    fn mpeg2_and_other_encoders() {
        // MPEG-2 Layer III, 64 kb/s, 22.05 kHz mono: 208 bytes, side info 9.
        let mut f = vec![0xFF, 0xF3, 0x80, 0xC0];
        f.resize(4 + 9, 0);
        f.extend_from_slice(b"Xing");
        f.resize(4 + 9 + 120, 0);
        f.extend_from_slice(b"GOGO1.0  ");
        f.resize(208, 0);
        f.extend_from_slice(&[0xFF, 0xF3, 0x80, 0xC0]);

        let got = first_frame(&mut Cursor::new(f)).unwrap();
        assert!(got.info_frame);
        assert_eq!(got.info_encoder, Some(InfoEncoder::Other));
        assert_eq!(got.frame_samples, 576);
    }

    #[test]
    fn no_frame_at_all() {
        assert_eq!(first_frame(&mut Cursor::new(vec![0u8; 4096])), None);
        assert_eq!(first_frame(&mut Cursor::new(b"ID3\x04".to_vec())), None);
    }
}
