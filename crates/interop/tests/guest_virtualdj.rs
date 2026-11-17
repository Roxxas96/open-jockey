//! Reads the committed guest-stick fixture's VirtualDJ branch.

use std::fs;
use std::path::{Component, Path, PathBuf};

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
    // The list is still read from MyLists, and Serato's source is untouched.
    assert_eq!(source.playlists.len(), 1);
    let serato = stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::Serato)
        .expect("a Serato source");
    assert_eq!(serato.tracks.len(), 3);
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

/// A temp copy of the fixture whose VirtualDJ list and database also name `outside` and a
/// `..` escape to it, each written as `spell` writes a path; returns the copy's root.
fn stick_with_outside(dir: &Path, outside: &Path, spell: fn(&Path) -> String) -> PathBuf {
    let root = dir.join("OJGUEST");
    copy_dir(&fixture(), &root);
    let list = root
        .join("VirtualDJ")
        .join("MyLists")
        .join("OJ VDJ A.vdjfolder");
    let escape = root.join("..").join("outside.wav");
    let mut text = fs::read_to_string(&list).unwrap();
    let entry = |p: &Path| format!("<song path=\"{}\" />\n", spell(p));
    let extra = format!("{}{}", entry(outside), entry(&escape));
    text = text.replace("</VirtualFolder>", &format!("{extra}</VirtualFolder>"));
    fs::write(&list, text).unwrap();
    let db = root.join("VirtualDJ").join("database.xml");
    let mut text = fs::read_to_string(&db).unwrap();
    let songs = format!(
        "<Song FilePath=\"{}\"><Tags Title=\"Evil\" /></Song>\n<Song FilePath=\"{}\"><Tags Title=\"Evil2\" /></Song>\n",
        spell(outside),
        spell(&escape)
    );
    text = text.replace(
        "</VirtualDJ_Database>",
        &format!("{songs}</VirtualDJ_Database>"),
    );
    fs::write(&db, text).unwrap();
    root
}

/// `path` as this platform writes it.
fn native(path: &Path) -> String {
    path.display().to_string()
}

/// `path` as VirtualDJ on Windows writes it: `C:/` and the parts after the drive or root.
fn drive_letter(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_owned()),
            _ => None,
        })
        .collect();
    format!("C:/{}", parts.join("/"))
}

const DROPPED: &str =
    "2 playlist entries in VirtualDJ/MyLists point at files that are not on this stick";

fn nothing_evil(source: &GuestSource) {
    for track in &source.tracks {
        let name = track.path.file_name().unwrap().to_string_lossy();
        assert!(!name.starts_with("outside"), "{}", track.path.display());
        assert_ne!(track.title.as_deref(), Some("Evil"));
        assert_ne!(track.title.as_deref(), Some("Evil2"));
    }
}

#[test]
fn a_path_outside_the_stick_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.wav");
    fs::copy(fixture().join("Serato").join("OJ Click 120.wav"), &outside).unwrap();
    let root = stick_with_outside(dir.path(), &outside, native);
    let stick = read_stick(&root, "OJGUEST");
    let source = vdj(&stick);
    nothing_evil(source);
    assert_eq!(source.playlists[0].tracks.len(), 3);
    let problem = source.problem.as_deref().expect("a problem");
    // A per-drive database's `X:/` is the drive's own root (VirtualDJ's convention), so on
    // Windows `C:\…\outside.wav` names `<stick>\…\outside.wav`: a song whose file is not on
    // the stick, skipped like any other missing one. Elsewhere the path is absolute, and
    // its file exists outside the stick.
    if cfg!(windows) {
        assert_eq!(problem, DROPPED);
    } else {
        assert_eq!(
            problem,
            format!("2 database songs are outside this stick; {DROPPED}")
        );
    }
}

/// The Windows case of [`a_path_outside_the_stick_is_never_read`], on every platform: the
/// paths are written `C:/…`, as VirtualDJ on Windows writes them.
#[test]
fn a_drive_letter_path_outside_the_stick_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.wav");
    fs::copy(fixture().join("Serato").join("OJ Click 120.wav"), &outside).unwrap();
    let root = stick_with_outside(dir.path(), &outside, drive_letter);
    let stick = read_stick(&root, "OJGUEST");
    let source = vdj(&stick);
    nothing_evil(source);
    assert_eq!(source.playlists[0].tracks.len(), 3);
    // The database's `C:/…` songs map onto the stick, where they are missing; the list
    // entries match no file on it. Neither reaches the file outside.
    assert_eq!(source.problem.as_deref(), Some(DROPPED));
    assert!(source.tracks.iter().all(|t| t.path.starts_with(&root)));
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_stick_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.wav");
    fs::copy(fixture().join("Serato").join("OJ Click 120.wav"), &outside).unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let link = root.join("VirtualDJ tracks").join("outside link.wav");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let list = root
        .join("VirtualDJ")
        .join("MyLists")
        .join("OJ VDJ A.vdjfolder");
    let text = fs::read_to_string(&list).unwrap().replace(
        "</VirtualFolder>",
        &format!("<song path=\"{}\" />\n</VirtualFolder>", link.display()),
    );
    fs::write(&list, text).unwrap();
    let stick = read_stick(&root, "OJGUEST");
    let source = vdj(&stick);
    assert!(
        source
            .tracks
            .iter()
            .all(|t| !t.path.ends_with("outside link.wav")),
        "symlink read"
    );
    assert_eq!(source.playlists[0].tracks.len(), 3);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("not on this stick"), "{problem}");
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
fn an_oversize_virtualdj_database_is_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    oversize(
        &root.join("VirtualDJ").join("database.xml"),
        256 * MIB + 1,
        b"<VirtualDJ_Database>",
    );
    let stick = read_stick(&root, "OJGUEST");
    let source = vdj(&stick);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("too large"), "{problem}");
    // The lists are still read.
    assert_eq!(source.playlists[0].tracks.len(), 3);
}

#[test]
fn oversize_virtualdj_lists_and_order_are_problems() {
    for name in ["big.vdjfolder", "big.m3u", "order"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("OJGUEST");
        copy_dir(&fixture(), &root);
        let lists = root.join("VirtualDJ").join("MyLists");
        oversize(&lists.join(name), 16 * MIB + 1, b"<VirtualFolder>\n");
        let stick = read_stick(&root, "OJGUEST");
        let source = vdj(&stick);
        let problem = source.problem.as_deref().expect("a problem");
        assert!(problem.contains("too large"), "{name}: {problem}");
        let a = source
            .playlists
            .iter()
            .find(|p| p.name == "OJ VDJ A")
            .unwrap();
        assert_eq!(a.tracks.len(), 3, "{name}");
    }
}

#[test]
fn a_list_path_with_very_many_parts_is_dropped_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OJGUEST");
    copy_dir(&fixture(), &root);
    let list = root
        .join("VirtualDJ")
        .join("MyLists")
        .join("OJ VDJ A.vdjfolder");
    let deep = format!("/{}VirtualDJ tracks/OJ Click 126.mp3", "a/".repeat(100_000));
    let text = fs::read_to_string(&list).unwrap().replace(
        "</VirtualFolder>",
        &format!("<song path=\"{deep}\" />\n</VirtualFolder>"),
    );
    fs::write(&list, text).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let at = root.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_stick(&at, "OJGUEST"));
    });
    let stick = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the stick is read in time");
    let source = vdj(&stick);
    assert_eq!(source.playlists[0].tracks.len(), 3);
    let problem = source.problem.as_deref().expect("a problem");
    assert!(problem.contains("not on this stick"), "{problem}");
}
