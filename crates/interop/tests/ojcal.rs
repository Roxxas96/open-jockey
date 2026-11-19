//! The committed OJCAL calibration stick: where Open Jockey's decode puts each file's first
//! click (see `fixtures/ojcal/README.md`).

use std::fs;
use std::path::{Path, PathBuf};

use interop::guest::{GuestApp, GuestTrack, read_stick};

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

/// Copies a folder tree, so a test reads a "mount" it may tamper with.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn track_of<'a>(tracks: &'a [GuestTrack], prefix: &str) -> &'a GuestTrack {
    tracks
        .iter()
        .find(|t| {
            t.path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("no track {prefix}"))
}

/// Where the first click sits in our decode, in seconds (the onset is 1 frame past it).
fn click_seconds(path: &Path) -> f64 {
    let rate = library::tags::read(path).sample_rate.unwrap();
    (onset_frames(path) - 1) as f64 / f64::from(rate)
}

#[test]
fn a_guest_ojcal_stick_puts_every_mark_on_its_click() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJCAL");
    copy_tree(&fixture(""), &root);
    let stick = read_stick(&root, "OJCAL");
    let source = |app| {
        stick
            .sources
            .iter()
            .find(|s| s.app == app)
            .unwrap_or_else(|| panic!("no source {app:?}"))
    };

    // Serato: hot cue 1 was placed by eye on the first click (files 01, 02, 03, 06, 07).
    let serato = source(GuestApp::Serato);
    for prefix in [
        "OJ Cal 01",
        "OJ Cal 02",
        "OJ Cal 03",
        "OJ Cal 06",
        "OJ Cal 07",
    ] {
        let track = track_of(&serato.tracks, prefix);
        let cue = track.hotcues.iter().find(|c| c.slot == 0).unwrap();
        let click = click_seconds(&track.path);
        let err = cue.seconds - click;
        println!(
            "serato {prefix}: cue {} click {click} err {err:+.4}",
            cue.seconds
        );
        assert!(err.abs() <= 0.002, "{prefix}: {err}");
    }

    // rekordbox: the first beat is the click, or a whole number of 0.5 s beats after it.
    for app in [GuestApp::RekordboxDevice, GuestApp::RekordboxXml] {
        let source = source(app);
        assert_eq!(source.tracks.len(), 8, "{app:?}: {:?}", source.problem);
        for track in &source.tracks {
            let first = track.first_beat.expect("a first beat");
            let click = click_seconds(&track.path);
            let beats = ((first - click) / 0.5).round();
            let err = first - click - beats * 0.5;
            println!(
                "{app:?} {:?}: beat {first} click {click} err {err:+.4}",
                track.path.file_name().unwrap()
            );
            assert!(first >= 0.0);
            assert!(err.abs() <= 0.0015, "{app:?} {:?}: {err}", track.path);
        }
    }
}

/// A copy of the guest fixture in a temp folder.
fn guest_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("guest");
    copy_tree(&from, &root);
    (dir, root)
}

#[test]
fn a_mark_at_the_apps_zero_is_kept_at_ours() {
    let (_dir, root) = guest_copy();
    let stick = read_stick(&root, "OJGUEST");
    for app in [GuestApp::RekordboxDevice, GuestApp::RekordboxXml] {
        let source = stick.sources.iter().find(|s| s.app == app).unwrap();
        let t128 = source
            .tracks
            .iter()
            .find(|t| t.title.as_deref() == Some("OJ Click 128"))
            .unwrap();
        assert_eq!(t128.main_cue, Some(0.0), "{app:?}");
    }
}

#[test]
fn an_unreadable_mp3_header_gets_no_shift() {
    let (_dir, root) = guest_copy();
    let mp3 = root.join("Serato").join("OJ Click 126.mp3");
    let want = {
        // The same cue read with the header intact is shifted by 50 ms.
        let stick = read_stick(&root, "OJGUEST");
        let s = stick
            .sources
            .iter()
            .find(|s| s.app == GuestApp::Serato)
            .unwrap();
        let t = s
            .tracks
            .iter()
            .find(|t| t.title.as_deref() == Some("OJ Click 126"))
            .unwrap();
        t.hotcues.iter().find(|c| c.slot == 0).unwrap().seconds + 0.050
    };
    // Zero the audio frames after the ID3 tag.
    let mut bytes = fs::read(&mp3).unwrap();
    let tag_len = 10
        + bytes[6..10]
            .iter()
            .fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7f));
    for b in &mut bytes[tag_len..] {
        *b = 0;
    }
    fs::write(&mp3, bytes).unwrap();
    let stick = read_stick(&root, "OJGUEST");
    let s = stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::Serato)
        .unwrap();
    let t =
        s.tracks.iter().find(|t| t.path == mp3).unwrap_or_else(|| {
            panic!("{:?}", s.tracks.iter().map(|t| &t.path).collect::<Vec<_>>())
        });
    let cue = t.hotcues.iter().find(|c| c.slot == 0).unwrap();
    assert!(
        (cue.seconds - want).abs() < 1e-6,
        "{} != {want}",
        cue.seconds
    );
}
