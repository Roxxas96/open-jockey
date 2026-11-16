//! Reads the committed guest-stick fixture's VirtualDJ branch.

use std::fs;
use std::path::{Path, PathBuf};

use interop::guest::{GuestApp, GuestSource, GuestStick, read_stick};

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

fn vdj(stick: &GuestStick) -> &GuestSource {
    stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::VirtualDj)
        .expect("a VirtualDJ source")
}

fn title_of(source: &GuestSource, index: usize) -> &str {
    source.tracks[index].title.as_deref().unwrap()
}

#[test]
fn virtualdj_branch_reads_the_list_in_order() {
    let stick = read_stick(&fixture(), "OJGUEST");
    assert_eq!(stick.name, "OJGUEST");
    let source = vdj(&stick);
    assert_eq!(source.problem, None);
    assert_eq!(source.playlists.len(), 1);
    let list = &source.playlists[0];
    assert_eq!(list.name, "OJ VDJ A");
    let titles: Vec<&str> = list.tracks.iter().map(|&i| title_of(source, i)).collect();
    assert_eq!(titles, ["OJ Click 126", "OJ Click 120", "OJ Click 128"]);
}

#[test]
fn virtualdj_cues_names_and_loop() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = vdj(&stick);
    let track = source
        .tracks
        .iter()
        .find(|t| t.title.as_deref() == Some("OJ Click 126"))
        .unwrap();
    assert!(track.path.starts_with(fixture()));
    assert!((track.bpm.unwrap() - 126.0).abs() < 0.01);
    assert_eq!(track.first_beat, None);
    let cue = |slot: u8| track.hotcues.iter().find(|c| c.slot == slot).unwrap();
    assert_eq!(track.hotcues.len(), 4);
    for (slot, seconds, name) in [
        (0, 10.554_195, "Test 1"),
        (1, 28.60, "TEst 2"),
        (2, 46.072_268, "Test 3"),
    ] {
        let c = cue(slot);
        assert!((c.seconds - seconds).abs() < 1e-6, "slot {slot}");
        assert_eq!(c.name.as_deref(), Some(name));
        assert_eq!(c.loop_seconds, None);
        assert_eq!(c.color, None);
    }
    let looped = cue(3);
    assert!((looped.seconds - 38.120).abs() < 1e-6);
    assert!((looped.loop_seconds.unwrap() - 60.0 / 126.0 * 8.0).abs() < 1e-3);
    assert_eq!(looped.color, None);
}

#[test]
fn a_broken_database_keeps_the_other_apps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let db = root.join("VirtualDJ").join("database.xml");
    let bytes = fs::read(&db).unwrap();
    fs::write(&db, &bytes[..bytes.len() / 3]).unwrap();

    let stick = read_stick(&root, "OJGUEST");
    let source = vdj(&stick);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("database.xml"), "{problem}");
    // The list is still read from MyLists; the Serato source joins this check with its reader.
    assert_eq!(source.playlists.len(), 1);
}

#[test]
fn paths_on_another_mount_name_map_to_the_stick() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("SOMETHING ELSE");
    copy_dir(&fixture(), &root);
    let stick = read_stick(&root, "SOMETHING ELSE");
    let source = vdj(&stick);
    let list = &source.playlists[0];
    assert_eq!(list.tracks.len(), 3);
    for &i in &list.tracks {
        let path = &source.tracks[i].path;
        assert!(path.starts_with(&root), "{}", path.display());
        assert!(path.is_file());
    }
}
