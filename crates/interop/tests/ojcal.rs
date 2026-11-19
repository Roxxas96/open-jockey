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

/// rekordbox's first beat (README) less the offset the rule gives for the file, unclamped.
const SHIFTED_FIRST_BEAT: [(&str, f64); 5] = [
    ("OJ Cal 01", 0.024 - 1105.0 / 44_100.0),
    ("OJ Cal 03", 0.051 - 2257.0 / 44_100.0),
    ("OJ Cal 04", 0.051 - 2257.0 / 44_100.0),
    ("OJ Cal 06", 0.046 - 2257.0 / 48_000.0),
    ("OJ Cal 07", 0.076 - 1681.0 / 22_050.0),
];

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
            // Not clamped: the grid keeps its phase, so the shifted first beat may be negative.
            // The shifted value is rekordbox's own (README) less the measured offset.
            let name = track
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let real = SHIFTED_FIRST_BEAT.iter().find(|(p, _)| name.starts_with(p));
            if let Some((_, want)) = real {
                assert!(
                    (first - want).abs() < 1e-9,
                    "{app:?} {name}: {first} != {want}"
                );
            }
            assert_eq!(beats, 0.0, "{app:?} {:?}: {first}", track.path);
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

#[test]
fn an_unreadable_mp3_header_gets_no_shift_in_rekordbox() {
    let (_dir, root) = guest_copy();
    let name = "OJ Click 126.mp3";
    let mut zeroed = 0;
    for entry in walk(&root) {
        if entry.file_name().is_some_and(|n| n == name) {
            let mut bytes = fs::read(&entry).unwrap();
            let tag_len = 10
                + bytes[6..10]
                    .iter()
                    .fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7f));
            for b in &mut bytes[tag_len..] {
                *b = 0;
            }
            fs::write(&entry, bytes).unwrap();
            zeroed += 1;
        }
    }
    assert!(zeroed > 0);
    let stick = read_stick(&root, "OJGUEST");
    for app in [GuestApp::RekordboxDevice, GuestApp::RekordboxXml] {
        let source = stick.sources.iter().find(|s| s.app == app).unwrap();
        let t = source
            .tracks
            .iter()
            .find(|t| t.title.as_deref() == Some("OJ Click 126") || t.path.ends_with(name))
            .unwrap_or_else(|| panic!("{app:?}: no 126"));
        // rekordbox's own 24 ms, as stored.
        let first = t.first_beat.unwrap();
        assert!((first - 0.024).abs() < 0.0006, "{app:?}: {first}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        if entry.file_type().unwrap().is_dir() {
            out.extend(walk(&entry.path()));
        } else {
            out.push(entry.path());
        }
    }
    out
}

// ---- Export: what each app wrote, written back ----

use std::collections::BTreeMap;

use interop::export::serato::tags::write_tags;
use interop::export::timing::{App, app_offset_seconds};
use interop::export::{ExportControl, ExportPlan, ExportTrack, FileKind, gather, plan_export, run};
use library::{HotcuePoint, Library, RawStatement, WriteOp, add_folder, scan_folder};
use rusqlite::types::Value;

/// The audio of the fixture: eight files, `OJ Cal 00` to `07`.
fn audio_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixture(""))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("OJ Cal ") && (n.ends_with(".mp3") || n.ends_with(".wav")))
        .collect();
    names.sort();
    assert_eq!(names.len(), 8);
    names
}

/// Serato's hot cue 1 (ms) for the files that have one (README).
const SERATO_CUE_MS: [(&str, f64); 5] = [
    ("OJ Cal 01", 51.0),
    ("OJ Cal 02", 27.0),
    ("OJ Cal 03", 51.0),
    ("OJ Cal 06", 50.0),
    ("OJ Cal 07", 77.0),
];

/// rekordbox's `TEMPO Inizio` (seconds) for every file (README).
const INIZIO: [(&str, f64); 8] = [
    ("OJ Cal 00", 0.001),
    ("OJ Cal 01", 0.024),
    ("OJ Cal 02", 0.024),
    ("OJ Cal 03", 0.051),
    ("OJ Cal 04", 0.051),
    ("OJ Cal 05", 0.024),
    ("OJ Cal 06", 0.046),
    ("OJ Cal 07", 0.076),
];

fn prefix_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy()[..9].to_owned()
}

struct Exported {
    _dir: tempfile::TempDir,
    stick: PathBuf,
    plan: ExportPlan,
    set: interop::export::ExportSet,
    music: PathBuf,
    /// Each file's click, in frames: the onset less the frame the 0.1 threshold adds.
    clicks: BTreeMap<String, u64>,
}

fn raw(lib: &Library, sql: &str, params: Vec<Value>) {
    lib.writer()
        .submit_wait(WriteOp::Raw(vec![RawStatement {
            sql: sql.into(),
            params,
        }]))
        .unwrap();
}

/// Gathers the eight fixture tracks from a library, gives each a hot cue and a first beat on
/// its click, and plans the export to an empty stick.
fn planned() -> Exported {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    let stick = dir.path().join("stick");
    fs::create_dir_all(&music).unwrap();
    fs::create_dir_all(&stick).unwrap();
    let mut clicks = BTreeMap::new();
    for name in audio_names() {
        fs::copy(fixture(&name), music.join(&name)).unwrap();
        clicks.insert(
            name[..9].to_owned(),
            onset_frames(&music.join(&name)).saturating_sub(1),
        );
    }
    let lib = Library::open(&dir.path().join("db").join("library.sqlite3")).unwrap();
    add_folder(&lib, &music).unwrap();
    scan_folder(&lib, &music, &mut |_| {}).unwrap();
    raw(
        &lib,
        "INSERT INTO playlists(name, position) VALUES ('Cal', 0)",
        vec![],
    );
    raw(
        &lib,
        "INSERT INTO playlist_tracks(playlist_id, position, track_id) \
         SELECT (SELECT id FROM playlists WHERE name = 'Cal'), ROW_NUMBER() OVER (ORDER BY path) - 1, id \
         FROM tracks",
        vec![],
    );
    let playlist: i64 = lib
        .reader()
        .unwrap()
        .connection()
        .query_row("SELECT id FROM playlists WHERE name = 'Cal'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let mut set = gather(&lib.reader().unwrap(), &[playlist]).unwrap();
    assert_eq!(set.tracks.len(), 8, "{:?}", set.skipped);
    for t in &mut set.tracks {
        let click = clicks[&prefix_of(&t.path)] as f64;
        t.bpm = Some(120.0);
        t.first_beat_frames = Some(click);
        t.hotcues[0] = Some(HotcuePoint {
            frames: click,
            loop_frames: None,
            color: None,
            name: None,
        });
    }
    let plan = plan_export(set.clone(), &stick, std::slice::from_ref(&music)).unwrap();
    Exported {
        _dir: dir,
        stick,
        plan,
        set,
        music,
        clicks,
    }
}

fn export_fixture() -> Exported {
    let e = planned();
    run(&e.plan, &|_| ExportControl::Go).unwrap();
    e
}

/// `Inizio` per file prefix, from the exported `rekordbox.xml`.
fn inizios(stick: &Path) -> BTreeMap<String, f64> {
    let text = fs::read_to_string(stick.join("Open Jockey").join("rekordbox.xml")).unwrap();
    let mut location = String::new();
    let mut out = BTreeMap::new();
    let mut reader = quick_xml::Reader::from_str(&text);
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                let attr = |k: &str| {
                    e.attributes().flatten().find_map(|a| {
                        (a.key.as_ref() == k).then(|| {
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned()
                        })
                    })
                };
                match e.name().as_ref() {
                    "TRACK" => {
                        if let Some(l) = attr("Location") {
                            location = l;
                        }
                    }
                    "TEMPO" => {
                        let name = percent_encoding::percent_decode_str(
                            location.rsplit('/').next().unwrap(),
                        )
                        .decode_utf8_lossy()
                        .into_owned();
                        out.insert(
                            name[..9].to_owned(),
                            attr("Inizio").unwrap().parse().unwrap(),
                        );
                    }
                    _ => {}
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// The exported copy of the file with this prefix.
fn copy_of(e: &Exported, prefix: &str) -> PathBuf {
    let p = e
        .plan
        .tracks
        .iter()
        .find(|p| prefix_of(&p.track.path) == prefix)
        .unwrap();
    e.stick
        .join(p.stick_path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Serato hot cue 1 (ms) as written into the exported copy: the guest reader's seconds with
/// the file's Serato offset put back.
fn serato_cue_ms(path: &Path) -> f64 {
    let root = path
        .ancestors()
        .find(|a| a.join("_Serato_").is_dir())
        .unwrap();
    let stick = read_stick(root, "OJCAL");
    let source = stick
        .sources
        .iter()
        .find(|s| s.app == GuestApp::Serato)
        .unwrap();
    let track = source.tracks.iter().find(|t| t.path == path).unwrap();
    let cue = track.hotcues.iter().find(|c| c.slot == 0).unwrap();
    let rate = library::tags::read(path).sample_rate.unwrap();
    let offset = app_offset_seconds(
        App::Serato,
        FileKind::Mp3,
        &decode::lead_in(path).unwrap(),
        rate,
    );
    ((cue.seconds + offset) * 1000.0).round()
}

#[test]
fn an_ojcal_export_writes_what_each_app_wrote() {
    let e = export_fixture();
    for (prefix, want) in SERATO_CUE_MS {
        let ms = serato_cue_ms(&copy_of(&e, prefix));
        println!("serato {prefix}: wrote {ms} ms, Serato wrote {want} ms");
        assert!((ms - want).abs() <= 2.0, "{prefix}: {ms} vs {want}");
    }
    let wrote = inizios(&e.stick);
    for (prefix, want) in INIZIO {
        let got = wrote[prefix];
        println!("rekordbox {prefix}: wrote {got} s, rekordbox wrote {want} s");
        assert!((got - want).abs() <= 0.0015, "{prefix}: {got} vs {want}");
    }
}

#[test]
fn export_then_guest_read_returns_the_same_frames() {
    let e = export_fixture();
    // rekordbox reads an XML at the stick's root: put the exported one there.
    fs::copy(
        e.stick.join("Open Jockey").join("rekordbox.xml"),
        e.stick.join("rekordbox.xml"),
    )
    .unwrap();
    let stick = read_stick(&e.stick, "OJCAL");
    for app in [GuestApp::Serato, GuestApp::RekordboxXml] {
        let source = stick
            .sources
            .iter()
            .find(|s| s.app == app)
            .unwrap_or_else(|| panic!("no source {app:?}"));
        assert_eq!(source.tracks.len(), 8, "{app:?}: {:?}", source.problem);
        for track in &source.tracks {
            let prefix = prefix_of(&track.path);
            let click = e.clicks[&prefix] as f64;
            let rate = f64::from(library::tags::read(&track.path).sample_rate.unwrap());
            // Serato rounds to 1 ms; rekordbox's seconds have 3 decimals and the XML 1.5 ms (R1).
            let tol = if app == GuestApp::Serato { 1.02 } else { 1.5 } * rate / 1000.0;
            let cue = track.hotcues.iter().find(|c| c.slot == 0).unwrap();
            let back = cue.seconds * rate;
            println!(
                "{app:?} {prefix}: cue {back:.1} first {:?} want {click}",
                track.first_beat.map(|f| f * rate)
            );
            assert!(
                (back - click).abs() <= tol,
                "{app:?} {prefix}: cue {back} vs {click}"
            );
            let first = track.first_beat.unwrap() * rate;
            assert!(
                (first - click).abs() <= tol,
                "{app:?} {prefix}: first {first} vs {click}"
            );
        }
    }
}

/// The bytes of every file under `dir`, by path relative to `dir`.
fn bytes_under(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    walk(dir)
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(dir).unwrap().to_path_buf(),
                fs::read(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn a_re_export_moves_only_mp3_serato_and_rekordbox_positions() {
    let e = export_fixture();
    let rate = |prefix: &str| -> f64 {
        f64::from(
            library::tags::read(&copy_of(&e, prefix))
                .sample_rate
                .unwrap(),
        )
    };

    // Put the stick back as an export made before this change left it: Serato's MP3 tags
    // with the old 1152-frame shift, rekordbox's MP3 positions with none.
    let new_xml = fs::read_to_string(e.stick.join("Open Jockey").join("rekordbox.xml")).unwrap();
    let mut mp3_copies = Vec::new();
    for planned in &e.plan.tracks {
        let t = &planned.track;
        if t.kind != FileKind::Mp3 {
            continue;
        }
        let copy = e.stick.join(
            planned
                .stick_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        let mut old: ExportTrack = t.clone();
        let click = old.first_beat_frames.unwrap();
        let old_frames = if t.lead_in.info_frame {
            click + 1152.0
        } else {
            click
        };
        old.lead_in = decode::LeadIn::default();
        old.first_beat_frames = Some(old_frames);
        old.hotcues[0] = Some(HotcuePoint {
            frames: old_frames,
            loop_frames: None,
            color: None,
            name: None,
        });
        write_tags(&copy, &old).unwrap();
        // rekordbox: the old export had no shift, so its seconds are the plain frames.
        let prefix = prefix_of(&t.path);
        let now = format!("{:.3}", inizios(&e.stick)[&prefix]);
        let was = format!("{:.3}", click / rate(&prefix));
        mp3_copies.push((copy, prefix, now, was));
    }
    // Rewrite the XML track by track: a track's own block holds its TEMPO and POSITION_MARKs.
    let mut out = String::new();
    let mut current: Option<(String, String)> = None;
    for line in new_xml.lines() {
        if line.contains("<TRACK ") && line.contains("Location=") {
            current = mp3_copies
                .iter()
                .find(|(_, prefix, ..)| line.contains(&prefix.replace(' ', "%20")))
                .map(|(_, _, now, was)| (now.clone(), was.clone()));
        }
        let mut line = line.to_owned();
        if let Some((now, was)) = &current {
            line = line
                .replace(&format!("Inizio=\"{now}\""), &format!("Inizio=\"{was}\""))
                .replace(&format!("Start=\"{now}\""), &format!("Start=\"{was}\""));
        }
        out.push_str(&line);
        out.push('\n');
    }
    let old_xml = out;
    assert_ne!(old_xml, new_xml);
    fs::write(e.stick.join("Open Jockey").join("rekordbox.xml"), &old_xml).unwrap();
    let old_cue = serato_cue_ms(&mp3_copies[0].0);
    assert!(old_cue > 1000.0 * e.clicks[&mp3_copies[0].1] as f64 / rate(&mp3_copies[0].1) + 20.0);

    let untouched = |stick: &Path| {
        let mut all = bytes_under(&stick.join("VirtualDJ"));
        for p in &e.plan.tracks {
            if p.track.kind == FileKind::Wav {
                let rel = PathBuf::from(&p.stick_path);
                all.insert(rel.clone(), fs::read(stick.join(&rel)).unwrap());
            }
        }
        all
    };
    let before = untouched(&e.stick);
    let again = plan_export(e.set.clone(), &e.stick, std::slice::from_ref(&e.music)).unwrap();
    let report = run(&again, &|_| ExportControl::Go).unwrap();
    assert_eq!(report.copied, 0);
    assert_eq!(
        report.retagged, 5,
        "the MP3s with an Info frame are retagged, the rest are not"
    );

    // The new values are back.
    let wrote = inizios(&e.stick);
    for (prefix, want) in INIZIO {
        assert!((wrote[prefix] - want).abs() <= 0.0015, "{prefix}");
    }
    for (prefix, want) in SERATO_CUE_MS {
        assert!(
            (serato_cue_ms(&copy_of(&e, prefix)) - want).abs() <= 2.0,
            "{prefix}"
        );
    }
    assert_eq!(
        new_xml,
        fs::read_to_string(e.stick.join("Open Jockey").join("rekordbox.xml")).unwrap()
    );
    // And nothing else moved.
    assert_eq!(before, untouched(&e.stick));
}
