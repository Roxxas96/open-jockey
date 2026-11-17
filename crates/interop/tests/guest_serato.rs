//! Reads the committed guest-stick fixture's Serato branch.

use std::fs;
use std::path::{Path, PathBuf};

use interop::ExportError;
use interop::export::serato::tags::write_tags;
use interop::export::serato::tlv::{self, Body, Record};
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

#[test]
fn a_serato_path_outside_the_stick_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.wav");
    fs::copy(fixture().join("Serato").join("OJ Click 120.wav"), &outside).unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let absolute = outside.to_string_lossy().into_owned();
    let paths = [
        "../outside.wav",
        absolute.as_str(),
        "Serato/OJ Click 126.mp3",
    ];
    let crate_file = root
        .join("_Serato_")
        .join("Subcrates")
        .join("OJ Serato B.crate");
    fs::write(&crate_file, tlv::crate_file(&paths)).unwrap();
    let otrk = |p: &str| Record {
        tag: *b"otrk",
        body: Body::Records(vec![tlv::text("pfil", p), tlv::text("tsng", "Evil")]),
    };
    let database = root.join("_Serato_").join("database V2");
    let mut bytes = fs::read(&database).unwrap();
    bytes.extend(tlv::encode(&[otrk("../outside.wav"), otrk(&absolute)]));
    fs::write(&database, bytes).unwrap();

    let stick = read_stick(&root, "OJGUEST");
    let source = serato(&stick);
    for track in &source.tracks {
        assert!(track.path.starts_with(&root), "{}", track.path.display());
        assert_ne!(track.title.as_deref(), Some("Evil"));
    }
    assert_eq!(source.tracks.len(), 3);
    let b = source
        .playlists
        .iter()
        .find(|p| p.name == "OJ Serato B")
        .unwrap();
    assert_eq!(b.tracks.len(), 1);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("outside this stick"), "{problem}");
}

#[cfg(unix)]
#[test]
fn a_metadata_symlink_out_of_the_stick_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let serato_dir = root.join("_Serato_");
    // The database and one crate are links to copies outside the stick.
    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let database = serato_dir.join("database V2");
    fs::rename(&database, elsewhere.join("database V2")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("database V2"), &database).unwrap();
    let crate_file = serato_dir.join("Subcrates").join("OJ Serato B.crate");
    fs::rename(&crate_file, elsewhere.join("B.crate")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("B.crate"), &crate_file).unwrap();
    // VirtualDJ's database too.
    let vdj = root.join("VirtualDJ").join("database.xml");
    fs::rename(&vdj, elsewhere.join("database.xml")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("database.xml"), &vdj).unwrap();

    let stick = read_stick(&root, "OJGUEST");
    let source = serato(&stick);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("database V2"), "{problem}");
    assert!(problem.contains("OJ Serato B.crate"), "{problem}");
    let names: Vec<&str> = source.playlists.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["OJ Serato A"]);
    let vdj_source = stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::VirtualDj)
        .unwrap();
    let problem = vdj_source.problem.as_deref().expect("a problem");
    assert!(problem.contains("database.xml"), "{problem}");

    // A `_Serato_` folder that is itself a link out of the stick is not read at all.
    let linked = dir.path().join("LINKED");
    copy_dir(&fixture(), &linked);
    let real = dir.path().join("elsewhere").join("_Serato_");
    fs::rename(linked.join("_Serato_"), &real).unwrap();
    std::os::unix::fs::symlink(&real, linked.join("_Serato_")).unwrap();
    let stick = read_stick(&linked, "LINKED");
    let source = serato(&stick);
    assert!(source.tracks.is_empty() && source.playlists.is_empty());
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("outside this stick"), "{problem}");
}

/// Writes `head` at the start of `path` and stretches the file to `len` bytes (sparse where
/// the file system allows), so a size cap can be tested without writing the bytes.
fn oversize(path: &Path, len: u64, head: &[u8]) {
    fs::write(path, head).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(len)
        .unwrap();
}

const MIB: u64 = 1024 * 1024;

#[test]
fn oversize_serato_files_are_problems() {
    // A record header claiming a huge body, so a read past the cap fails fast, not slowly.
    let head = [b'o', b't', b'r', b'k', 0x7F, 0xFF, 0xFF, 0xFF];
    let cases = [
        (
            ["_Serato_", "database V2"].as_slice(),
            256 * MIB + 1,
            "database V2",
        ),
        (
            ["_Serato_", "Subcrates", "OJ Serato B.crate"].as_slice(),
            16 * MIB + 1,
            "OJ Serato B.crate",
        ),
        (
            ["_Serato_", "neworder.pref"].as_slice(),
            16 * MIB + 1,
            "neworder.pref",
        ),
    ];
    for (parts, len, name) in cases {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("OJGUEST");
        copy_dir(&fixture(), &root);
        let path = parts.iter().fold(root.clone(), |p, part| p.join(part));
        oversize(&path, len, &head);
        let stick = read_stick(&root, "OJGUEST");
        let source = serato(&stick);
        let problem = source.problem.as_deref().expect("a problem");
        assert!(problem.contains(name), "{name}: {problem}");
        assert!(problem.contains("too large"), "{name}: {problem}");
        // Crate A is still read.
        assert!(source.playlists.iter().any(|p| p.name == "OJ Serato A"));
    }
}

#[test]
fn serato_grid_from_the_tempo_map() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = serato(&stick);
    // 120.wav: the map's first beat (the audio's second click, at 0.5 s) and its BPM.
    let wav = track_named(source, "OJ Click 120");
    assert!((wav.first_beat.unwrap() - 0.499_423_485).abs() < 1e-6);
    assert!((wav.bpm.unwrap() - 119.995_584).abs() < 1e-5);
    // 126.mp3: the map's seconds less the Info-frame shift, as for its cues.
    let mp3 = track_named(source, "OJ Click 126");
    let rate = f64::from(
        library::tags::read(&fixture().join("Serato").join("OJ Click 126.mp3"))
            .sample_rate
            .unwrap(),
    );
    let want = 0.531_400_496 - 1152.0 / rate;
    assert!((mp3.first_beat.unwrap() - want).abs() < 1e-6);
    assert!((mp3.bpm.unwrap() - 126.000_728).abs() < 1e-5);
}

#[test]
fn serato_mp3_cues_match_what_serato_wrote() {
    // The README's 126.mp3 cue 1: 7954 ms as Serato stored it, read back less the
    // Info-frame shift. Pins the shift's direction against a file Serato wrote.
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = serato(&stick);
    let rate = f64::from(
        library::tags::read(&fixture().join("Serato").join("OJ Click 126.mp3"))
            .sample_rate
            .unwrap(),
    );
    let track = track_named(source, "OJ Click 126");
    for (slot, ms, color) in [
        (0, 7954.0, 0xCC0000),
        (1, 29418.0, 0xCC8800),
        (2, 44451.0, 0x0000CC),
    ] {
        let cue = track.hotcues.iter().find(|c| c.slot == slot).unwrap();
        let want = ms / 1000.0 - 1152.0 / rate;
        assert!(
            (cue.seconds - want).abs() < 1e-6,
            "slot {slot}: {}",
            cue.seconds
        );
        assert_eq!(cue.color, Some(color), "slot {slot}");
    }
}

#[test]
fn serato_grid_from_a_beatgrid_tag() {
    // Export writes `Serato BeatGrid` and drops the other Serato frames (the tempo map).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    for (file, kind) in [
        ("OJ Click 120.wav", FileKind::Wav),
        ("OJ Click 126.mp3", FileKind::Mp3),
    ] {
        let path = root.join("Serato").join(file);
        let rate = library::tags::read(&path).sample_rate.unwrap();
        let t = ExportTrack {
            id: 1,
            path: path.clone(),
            fingerprint: [0; 32],
            size: 1,
            title: file.into(),
            artist: None,
            album: None,
            genre: None,
            sample_rate: rate,
            duration_frames: None,
            bpm: Some(124.5),
            first_beat_frames: Some(0.25 * f64::from(rate)),
            key: None,
            cue_frames: None,
            hotcues: Default::default(),
            kind,
            lead_in: decode::lead_in(&path).unwrap(),
        };
        write_tags(&path, &t)
            .map_err(|e: ExportError| e.to_string())
            .unwrap();
    }
    let stick = read_stick(&root, "OJGUEST");
    let source = serato(&stick);
    for title in ["OJ Click 120", "OJ Click 126"] {
        let track = track_named(source, title);
        assert!((track.bpm.unwrap() - 124.5).abs() < 1e-4, "{title}");
        let first = track.first_beat.unwrap();
        assert!((first - 0.25).abs() < 0.001, "{title}: {first}");
    }
}

#[test]
fn serato_entries_for_missing_files_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    fs::remove_file(root.join("Serato").join("OJ Click 126.mp3")).unwrap();
    let stick = read_stick(&root, "OJGUEST");
    let source = serato(&stick);
    assert_eq!(source.tracks.len(), 2);
    let b = source
        .playlists
        .iter()
        .find(|p| p.name == "OJ Serato B")
        .unwrap();
    assert!(b.tracks.is_empty());
    let problem = source.problem.as_deref().expect("a problem");
    // The database's 126 and its entries in crates A and B.
    assert!(
        problem.contains("1 database V2 track is not on this stick"),
        "{problem}"
    );
    assert!(
        problem.contains("2 crate entries point at files that are not on this stick"),
        "{problem}"
    );
    assert!(!problem.contains("outside"), "{problem}");
}
