//! Reads the committed guest-stick fixture's rekordbox device library (`export.pdb` and `ANLZ`).

use std::fs;
use std::path::{Path, PathBuf};

use interop::guest::{GuestApp, GuestSource, GuestStick, GuestTrack, read_stick};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("guest")
}

fn source_of(stick: &GuestStick, app: GuestApp) -> &GuestSource {
    stick
        .sources
        .iter()
        .find(|s| s.app == app)
        .expect("a source for the app")
}

fn by_title<'a>(source: &'a GuestSource, title: &str) -> &'a GuestTrack {
    source
        .tracks
        .iter()
        .find(|t| t.title.as_deref() == Some(title))
        .unwrap()
}

/// Copies a folder tree, for a "mount" a test may tamper with.
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

#[test]
fn device_playlist_in_order() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = source_of(&stick, GuestApp::RekordboxDevice);
    assert_eq!(source.problem, None);
    assert_eq!(source.playlists.len(), 1);
    let list = &source.playlists[0];
    assert_eq!(list.name, "OJ Rekordbox A");
    let titles: Vec<&str> = list
        .tracks
        .iter()
        .map(|&i| source.tracks[i].title.as_deref().unwrap())
        .collect();
    assert_eq!(titles, ["OJ Click 120", "OJ Click 128", "OJ Click 126"]);
    for track in &source.tracks {
        assert!(track.path.starts_with(fixture().join("Contents")));
        assert!(track.path.is_file());
        assert_eq!(track.artist.as_deref(), Some("Open Jockey Fixture"));
        assert_eq!(track.album.as_deref(), Some("OJ Fixtures"));
    }
}

#[test]
fn device_grid_from_anlz() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = source_of(&stick, GuestApp::RekordboxDevice);
    let bpm = |title| by_title(source, title).bpm.unwrap();
    assert!((bpm("OJ Click 120") - 149.96).abs() < 1e-9);
    assert!((bpm("OJ Click 126") - 126.0).abs() < 1e-9);
    assert!((bpm("OJ Click 128") - 128.0).abs() < 1e-9);
    for track in &source.tracks {
        assert!(track.first_beat.is_some_and(|b| (0.0..1.0).contains(&b)));
    }
}

#[test]
fn device_cues_match_the_xml() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let device = source_of(&stick, GuestApp::RekordboxDevice);
    let xml = source_of(&stick, GuestApp::RekordboxXml);
    for title in ["OJ Click 120", "OJ Click 126", "OJ Click 128"] {
        let (d, x) = (by_title(device, title), by_title(xml, title));
        assert_eq!(d.hotcues.len(), x.hotcues.len(), "{title}");
        for (dc, xc) in d.hotcues.iter().zip(&x.hotcues) {
            assert_eq!(dc.slot, xc.slot, "{title}");
            assert!((dc.seconds - xc.seconds).abs() < 0.001, "{title}");
            // A cue left uncoloured has no palette code on the device: it carries the player's
            // LED colour, where rekordbox.xml has the app's default for that slot.
            let led = match (title, dc.slot) {
                ("OJ Click 126" | "OJ Click 128", 0) => Some(0xff0017),
                ("OJ Click 126", 1) => Some(0x00c4ff),
                ("OJ Click 126" | "OJ Click 128", 2) => Some(0x33ff00),
                _ => None,
            };
            assert_eq!(
                dc.color,
                led.map(Some).unwrap_or(xc.color),
                "{title} slot {}",
                dc.slot
            );
            assert_eq!(dc.name, xc.name, "{title} slot {}", dc.slot);
            match (dc.loop_seconds, xc.loop_seconds) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 0.001, "{title}"),
                (a, b) => assert_eq!(a, b, "{title}"),
            }
        }
        match (d.main_cue, x.main_cue) {
            (Some(a), Some(b)) => assert!((a - b).abs() < 0.001, "{title}"),
            (a, b) => assert_eq!(a, b, "{title}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn a_device_path_outside_the_stick_is_never_read() {
    // The `.DAT` of the 126 track is a link to a file outside the stick: it is refused, counted
    // in `problem`, and the track keeps no grid or cues from it.
    let stick = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    copy_tree(&fixture().join("PIONEER"), &stick.path().join("PIONEER"));
    copy_tree(&fixture().join("Contents"), &stick.path().join("Contents"));
    let dat = stick
        .path()
        .join("PIONEER")
        .join("USBANLZ")
        .join("P030")
        .join("00006630")
        .join("ANLZ0000.DAT");
    let elsewhere = outside.path().join("ANLZ0000.DAT");
    fs::copy(&dat, &elsewhere).unwrap();
    fs::remove_file(&dat).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dat).unwrap();

    let read = read_stick(stick.path(), "COPY");
    let source = source_of(&read, GuestApp::RekordboxDevice);
    let problem = source.problem.as_deref().expect("a problem is reported");
    assert!(problem.contains("outside this stick"), "{problem}");
    let moved: Vec<_> = source
        .tracks
        .iter()
        .filter(|t| t.first_beat.is_none() && t.hotcues.is_empty())
        .collect();
    assert_eq!(moved.len(), 1);
    assert_eq!(source.tracks.len(), 3);
}

/// A temp stick with the fixture's device library, and the path of its `export.pdb`.
fn device_copy() -> (tempfile::TempDir, PathBuf) {
    let stick = tempfile::tempdir().unwrap();
    copy_tree(&fixture().join("PIONEER"), &stick.path().join("PIONEER"));
    copy_tree(&fixture().join("Contents"), &stick.path().join("Contents"));
    let pdb = stick
        .path()
        .join("PIONEER")
        .join("rekordbox")
        .join("export.pdb");
    (stick, pdb)
}

#[test]
fn a_corrupt_export_pdb_sets_a_problem() {
    let original = fs::read(
        fixture()
            .join("PIONEER")
            .join("rekordbox")
            .join("export.pdb"),
    )
    .unwrap();
    // A pseudo-random sequence, so the run is repeatable.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut noise = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect()
    };
    // Cut after the second page: the tables' pages are gone.
    let page_size = u32::from_le_bytes(original[4..8].try_into().unwrap()) as usize;
    let truncated = original[..2 * page_size].to_vec();
    let mut header_cut = original.clone();
    header_cut.truncate(20);
    let random = noise(original.len());
    // The real header over random pages.
    let mut scrambled = original.clone();
    let tail = noise(scrambled.len() - 4096);
    scrambled[4096..].copy_from_slice(&tail);
    for (label, bytes, must_report) in [
        ("truncated", truncated, true),
        ("header cut", header_cut, true),
        ("random", random, true),
        ("scrambled pages", scrambled, false),
    ] {
        let (stick, pdb) = device_copy();
        fs::write(&pdb, bytes).unwrap();
        let read = read_stick(stick.path(), "COPY");
        let source = source_of(&read, GuestApp::RekordboxDevice);
        if must_report {
            assert!(source.problem.is_some(), "{label}");
        }
    }
}

#[test]
fn a_page_cycle_in_export_pdb_terminates() {
    let (stick, pdb) = device_copy();
    let mut bytes = fs::read(&pdb).unwrap();
    let le = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let page_size = le(&bytes, 4) as usize;
    for t in 0..le(&bytes, 8) as usize {
        let at = 28 + 16 * t;
        let first = le(&bytes, at + 8) as usize;
        // No table ends where it says, and every page chains back to its own table's first.
        bytes[at + 12..at + 16].copy_from_slice(&0x00ff_fff0u32.to_le_bytes());
        let page = first * page_size;
        bytes[page + 12..page + 16].copy_from_slice(&(first as u32).to_le_bytes());
    }
    fs::write(&pdb, bytes).unwrap();
    let read = read_stick(stick.path(), "COPY");
    let _ = source_of(&read, GuestApp::RekordboxDevice);
}
