//! The committed OJCAL calibration stick: where Open Jockey's decode puts each file's first
//! click (see `fixtures/ojcal/README.md`).

use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ojcal")
        .join(name)
}

/// The first frame of `path`'s decode with a sample louder than 0.1: the first click's onset.
fn onset_frames(path: &Path) -> u64 {
    let track = decode::decode_file(path).unwrap().track;
    (0..track.frames())
        .find(|&i| track.frame(i).iter().any(|s| s.abs() > 0.1))
        .map(|i| i as u64)
        .unwrap_or_else(|| panic!("no click in {}", path.display()))
}

/// Files with a gapless header (and the WAV): the click is at frame 0.
const TRIMMED: [&str; 6] = [
    "OJ Cal 00 reference 44k.wav",
    "OJ Cal 01 ffmpeg 44k CBR.mp3",
    "OJ Cal 03 lame 44k CBR.mp3",
    "OJ Cal 04 lame 44k VBR.mp3",
    "OJ Cal 06 lame 48k CBR.mp3",
    "OJ Cal 07 lame 22k CBR.mp3",
];

/// Files without one: nothing is trimmed, so the click sits behind the encoder and decoder
/// delay (576 + 529 frames).
const UNTRIMMED: [&str; 2] = [
    "OJ Cal 02 ffmpeg 44k no-Info.mp3",
    "OJ Cal 05 lame 44k no-Info.mp3",
];

#[test]
fn ojcal_onsets_in_our_decode() {
    for name in TRIMMED {
        let onset = onset_frames(&fixture(name));
        println!("{name}: onset {onset} frames");
        assert!(onset <= 2, "{name}: onset {onset} frames");
    }
    for name in UNTRIMMED {
        let onset = onset_frames(&fixture(name));
        println!("{name}: onset {onset} frames");
        assert!(onset.abs_diff(1105) <= 2, "{name}: onset {onset} frames");
    }
}
