//! Reads the committed guest-stick fixture's Serato branch.

use std::fs;
use std::path::{Path, PathBuf};

use interop::ExportError;
use interop::export::serato::tags::write_tags;
use interop::export::{ExportTrack, FileKind};
use interop::guest::{GuestApp, GuestSource, GuestStick, read_stick};
use library::HotcuePoint;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("guest")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn serato(stick: &GuestStick) -> &GuestSource {
    stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::Serato)
        .expect("a Serato source")
}

fn titles(source: &GuestSource, tracks: &[usize]) -> Vec<String> {
    tracks
        .iter()
        .map(|&i| source.tracks[i].title.clone().unwrap())
        .collect()
}

fn track_named<'a>(source: &'a GuestSource, title: &str) -> &'a interop::guest::GuestTrack {
    source
        .tracks
        .iter()
        .find(|t| t.title.as_deref() == Some(title))
        .unwrap_or_else(|| panic!("no track {title}"))
}

#[test]
fn serato_crates_in_order() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = serato(&stick);
    assert_eq!(source.problem, None);
    let names: Vec<&str> = source.playlists.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["OJ Serato A", "OJ Serato B"]);
    assert_eq!(
        titles(source, &source.playlists[0].tracks),
        ["OJ Click 128", "OJ Click 120", "OJ Click 126"]
    );
    assert_eq!(
        titles(source, &source.playlists[1].tracks),
        ["OJ Click 126"]
    );
    assert_eq!(source.tracks.len(), 3);
    let track = track_named(source, "OJ Click 120");
    assert!(track.path.starts_with(fixture()));
    assert!(track.path.is_file());
    assert_eq!(track.artist.as_deref(), Some("Open Jockey Fixture"));
}

#[test]
fn serato_cues_from_the_files() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = serato(&stick);
    let track = track_named(source, "OJ Click 120");
    assert_eq!(track.hotcues.len(), 4);
    for (slot, seconds, color) in [
        (0, 10.944, 0xCC0000),
        (1, 26.161, 0xCC8800),
        (2, 44.277, 0x0000CC),
        (3, 35.786, 0xCCCC00),
    ] {
        let cue = track.hotcues.iter().find(|c| c.slot == slot).unwrap();
        assert!((cue.seconds - seconds).abs() < 1e-6, "slot {slot}");
        assert_eq!(cue.color, Some(color), "slot {slot}");
        assert_eq!(cue.loop_seconds, None);
        assert_eq!(cue.name, None);
    }
}

#[test]
fn serato_mp3_offset_round_trips_with_export() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let mp3 = root.join("Serato").join("OJ Click 126.mp3");
    let rate = library::tags::read(&mp3).sample_rate.unwrap();
    let lead_in = decode::lead_in(&mp3).unwrap();
    assert!(lead_in.gapless_header, "the fixture MP3 has an Info frame");

    let mut hotcues: [Option<HotcuePoint>; 8] = Default::default();
    hotcues[0] = Some(HotcuePoint {
        frames: 5.0 * f64::from(rate),
        loop_frames: None,
        color: Some(0xCC0000),
        name: Some("Drop".into()),
    });
    hotcues[1] = Some(HotcuePoint {
        frames: 8.0 * f64::from(rate),
        loop_frames: Some(2.0 * f64::from(rate)),
        color: None,
        name: None,
    });
    let t = ExportTrack {
        id: 1,
        path: mp3.clone(),
        fingerprint: [0; 32],
        size: 1,
        title: "OJ Click 126".into(),
        artist: None,
        album: None,
        genre: None,
        sample_rate: rate,
        duration_frames: None,
        bpm: None,
        first_beat_frames: None,
        key: None,
        cue_frames: None,
        hotcues,
        kind: FileKind::Mp3,
        lead_in,
    };
    write_tags(&mp3, &t)
        .map_err(|e: ExportError| e.to_string())
        .unwrap();

    let stick = read_stick(&root, "OJGUEST");
    let track = track_named(serato(&stick), "OJ Click 126");
    let cue = track.hotcues.iter().find(|c| c.slot == 0).unwrap();
    assert!((cue.seconds - 5.0).abs() < 0.001, "{}", cue.seconds);
    assert_eq!(cue.name.as_deref(), Some("Drop"));
    let looped = track.hotcues.iter().find(|c| c.slot == 1).unwrap();
    assert!((looped.seconds - 8.0).abs() < 0.001, "{}", looped.seconds);
    assert!((looped.loop_seconds.unwrap() - 2.0).abs() < 0.001);
}

#[test]
fn a_file_with_no_markers_has_no_cues() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    // A WAV with no id3 chunk at all: the RIFF header and the audio chunks only.
    let wav = root.join("Serato").join("OJ Click 120.wav");
    let bytes = fs::read(&wav).unwrap();
    let mut out = bytes[..12].to_vec();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = (at + 8 + len + (len & 1)).min(bytes.len());
        if !id.eq_ignore_ascii_case(b"id3 ") {
            out.extend_from_slice(&bytes[at..end]);
        }
        at = end;
    }
    let riff = u32::try_from(out.len() - 8).unwrap().to_le_bytes();
    out[4..8].copy_from_slice(&riff);
    fs::write(&wav, out).unwrap();

    let stick = read_stick(&root, "OJGUEST");
    let source = serato(&stick);
    let track = track_named(source, "OJ Click 120");
    assert!(track.hotcues.is_empty());
    // The other files keep theirs.
    assert!(!track_named(source, "OJ Click 126").hotcues.is_empty());
}
