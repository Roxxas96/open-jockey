//! Reads the committed guest-stick fixture's `rekordbox.xml` branch.

use std::fs;
use std::path::{Path, PathBuf};

use interop::guest::{GuestApp, GuestSource, GuestStick, read_stick};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("guest")
}

fn xml(stick: &GuestStick) -> &GuestSource {
    stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::RekordboxXml)
        .expect("a rekordbox.xml source")
}

fn title_of(source: &GuestSource, index: usize) -> &str {
    source.tracks[index].title.as_deref().unwrap()
}

fn by_title<'a>(source: &'a GuestSource, title: &str) -> &'a interop::guest::GuestTrack {
    source
        .tracks
        .iter()
        .find(|t| t.title.as_deref() == Some(title))
        .unwrap()
}

/// A temp stick holding the fixture's 126 mp3 under `Music/` and the given `rekordbox.xml`.
fn stick_with(xml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("Music");
    fs::create_dir_all(&music).unwrap();
    fs::copy(
        fixture().join("VirtualDJ tracks").join("OJ Click 126.mp3"),
        music.join("OJ Click 126.mp3"),
    )
    .unwrap();
    fs::write(dir.path().join("rekordbox.xml"), xml).unwrap();
    dir
}

fn one_track_xml(location: &str, playlists: &str) -> String {
    format!(
        r#"<DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="1">
<TRACK TrackID="1" Name="Inline" Location="{location}"><TEMPO Inizio="0.5" Bpm="126.00"/></TRACK>
</COLLECTION><PLAYLISTS>{playlists}</PLAYLISTS></DJ_PLAYLISTS>"#
    )
}

#[test]
fn rekordbox_xml_playlist_in_order() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = xml(&stick);
    assert_eq!(source.playlists.len(), 1);
    let list = &source.playlists[0];
    assert_eq!(list.name, "OJ Rekordbox A");
    let titles: Vec<&str> = list.tracks.iter().map(|&i| title_of(source, i)).collect();
    assert_eq!(titles, ["OJ Click 120", "OJ Click 128", "OJ Click 126"]);
    for track in &source.tracks {
        assert!(track.path.starts_with(fixture()));
        assert!(track.path.is_file());
    }
}

#[test]
fn rekordbox_xml_marks() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = xml(&stick);
    let t120 = by_title(source, "OJ Click 120");
    let a = &t120.hotcues[0];
    assert_eq!(a.slot, 0);
    assert!((a.seconds - 5.216).abs() < 1e-9);
    assert_eq!(a.name.as_deref(), Some("Test 3"));
    assert_eq!(a.color, Some(0xB4BE04));
    let d = t120.hotcues.iter().find(|c| c.slot == 3).unwrap();
    assert!((d.seconds - 30.422).abs() < 1e-9);
    assert!((d.loop_seconds.unwrap() - 1.601).abs() < 1e-9);
    assert_eq!(d.color, Some(0xFF8C00));
    assert_eq!(d.name, None);
    let t128 = by_title(source, "OJ Click 128");
    assert_eq!(t128.main_cue, Some(0.0));
    assert_eq!(t128.hotcues.len(), 3);
}

#[test]
fn rekordbox_xml_uses_its_grid_even_when_wrong() {
    let stick = read_stick(&fixture(), "OJGUEST");
    let source = xml(&stick);
    let t120 = by_title(source, "OJ Click 120");
    assert_eq!(t120.bpm, Some(149.96));
    assert_eq!(t120.first_beat, Some(0.015));
    // Only the first TEMPO counts.
    let t126 = by_title(source, "OJ Click 126");
    assert_eq!(t126.first_beat, Some(0.024));
}

#[test]
fn nested_folders_are_flattened() {
    let playlists = r#"<NODE Type="0" Name="ROOT" Count="2">
<NODE Type="0" Name="Sets" Count="1"><NODE Type="1" Name="Friday" KeyType="0" Entries="1"><TRACK Key="1"/></NODE></NODE>
<NODE Type="1" Name="Plain" KeyType="0" Entries="1"><TRACK Key="1"/></NODE></NODE>"#;
    let stick = stick_with(&one_track_xml(
        "file://localhost/Music/OJ%20Click%20126.mp3",
        playlists,
    ));
    let read = read_stick(stick.path(), "T");
    let source = xml(&read);
    assert_eq!(source.problem, None);
    let names: Vec<&str> = source.playlists.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Sets / Friday", "Plain"]);
    assert_eq!(source.tracks.len(), 1);
}

#[test]
fn a_rekordbox_xml_location_outside_the_stick_is_never_read() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.mp3");
    fs::write(&secret, b"not music").unwrap();
    let abs = secret.to_string_lossy().replace('\\', "/");
    let abs = format!("file://localhost/{}", abs.trim_start_matches('/'));
    let dir = stick_with("");
    let body = format!(
        r#"<DJ_PLAYLISTS><COLLECTION Entries="2">
<TRACK TrackID="1" Name="Abs" Location="{abs}"/>
<TRACK TrackID="2" Name="Dots" Location="file://localhost/Music/../../secret.mp3"/>
</COLLECTION><PLAYLISTS><NODE Type="0" Name="ROOT"><NODE Type="1" Name="P"><TRACK Key="1"/><TRACK Key="2"/></NODE></NODE></PLAYLISTS></DJ_PLAYLISTS>"#
    );
    fs::write(dir.path().join("rekordbox.xml"), body).unwrap();
    let read = read_stick(dir.path(), "T");
    let source = xml(&read);
    assert!(source.tracks.is_empty(), "{:?}", source.tracks);
    assert!(source.playlists.iter().all(|p| p.tracks.is_empty()));
    let problem = source.problem.as_deref().unwrap();
    assert!(problem.contains("outside this stick"), "{problem}");
    assert!(problem.contains("not found"), "{problem}");
}

#[test]
fn malformed_rekordbox_xml_sets_a_problem() {
    let dir = stick_with("<DJ_PLAYLISTS><COLLECTION><TRACK");
    let read = read_stick(dir.path(), "T");
    assert!(xml(&read).problem.is_some());
}

/// A temp stick with empty-ish files at the given relative paths and a one-track XML naming
/// `Name.mp3` somewhere off the stick.
fn stick_with_files(files: &[&str]) -> tempfile::TempDir {
    let dir = stick_with("");
    for rel in files {
        let path = rel
            .split('/')
            .fold(dir.path().to_path_buf(), |p, part| p.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(
            fixture().join("VirtualDJ tracks").join("OJ Click 126.mp3"),
            path,
        )
        .unwrap();
    }
    fs::remove_dir_all(dir.path().join("Music")).unwrap();
    fs::write(
        dir.path().join("rekordbox.xml"),
        one_track_xml("file://localhost/Elsewhere/t.mp3", ""),
    )
    .unwrap();
    dir
}

fn only_track_path(dir: &tempfile::TempDir) -> (Option<PathBuf>, Option<String>) {
    let read = read_stick(dir.path(), "T");
    let source = xml(&read);
    (
        source
            .tracks
            .first()
            .map(|t| t.path.strip_prefix(dir.path()).unwrap().to_path_buf()),
        source.problem.clone(),
    )
}

#[test]
fn the_normal_fixture_reads_without_a_problem() {
    let stick = read_stick(&fixture(), "OJGUEST");
    assert_eq!(xml(&stick).problem, None);
}

#[test]
fn a_true_tie_is_noted() {
    let dir = stick_with_files(&["B/t.mp3", "A/t.mp3"]);
    let (path, problem) = only_track_path(&dir);
    assert_eq!(path, Some(Path::new("A").join("t.mp3")));
    assert!(problem.unwrap().contains("several files"));
}

#[test]
fn contents_is_preferred_and_a_shorter_path_decides() {
    let dir = stick_with_files(&[
        "t.mp3",
        "Other/t.mp3",
        "Contents/X/Y/t.mp3",
        "Contents/Z/t.mp3",
    ]);
    let (path, problem) = only_track_path(&dir);
    assert_eq!(path, Some(["Contents", "Z", "t.mp3"].iter().collect()));
    assert_eq!(problem, None);
}

#[test]
fn the_name_search_is_bounded_in_depth_and_skips_hidden_folders() {
    let deep = stick_with_files(&["a/b/c/d/e/f/g/t.mp3"]);
    assert_eq!(only_track_path(&deep).0, None);
    let ok = stick_with_files(&["a/b/c/d/e/f/t.mp3"]);
    assert!(only_track_path(&ok).0.is_some());
    let hidden = stick_with_files(&[".trash/t.mp3"]);
    assert_eq!(only_track_path(&hidden).0, None);
}

#[test]
fn fades_and_load_markers_never_become_the_main_cue() {
    let body = r#"<DJ_PLAYLISTS><COLLECTION Entries="1">
<TRACK TrackID="1" Name="M" Location="file://localhost/Music/OJ%20Click%20126.mp3">
<POSITION_MARK Name="" Type="3" Start="1.0" Num="-1"/>
<POSITION_MARK Name="" Type="4" Start="2.0" End="3.0" Num="-1"/>
<POSITION_MARK Name="" Type="0" Start="4.0" Num="-1"/>
<POSITION_MARK Name="" Type="0" Start="5.0" Num="-1"/>
</TRACK></COLLECTION></DJ_PLAYLISTS>"#;
    let dir = stick_with(body);
    let read = read_stick(dir.path(), "T");
    let source = xml(&read);
    assert_eq!(source.tracks[0].main_cue, Some(4.0));
    assert!(source.tracks[0].hotcues.is_empty());
}

#[test]
fn a_truncated_rekordbox_xml_is_flagged() {
    let body = r#"<DJ_PLAYLISTS><COLLECTION Entries="1">
<TRACK TrackID="1" Name="M" Location="file://localhost/Music/OJ%20Click%20126.mp3">"#;
    let dir = stick_with(body);
    let read = read_stick(dir.path(), "T");
    assert!(xml(&read).problem.as_deref().unwrap().contains("truncated"));
}

#[test]
fn a_playlist_inside_a_playlist_keeps_the_outer_one() {
    let playlists = r#"<NODE Type="0" Name="ROOT"><NODE Type="1" Name="Outer"><TRACK Key="1"/>
<NODE Type="1" Name="Inner"><TRACK Key="1"/></NODE></NODE></NODE>"#;
    let stick = stick_with(&one_track_xml(
        "file://localhost/Music/OJ%20Click%20126.mp3",
        playlists,
    ));
    let read = read_stick(stick.path(), "T");
    let names: Vec<&str> = xml(&read)
        .playlists
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Outer", "Outer / Inner"]);
}
