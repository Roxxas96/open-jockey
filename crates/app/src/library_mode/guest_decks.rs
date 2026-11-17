//! Guest tracks on the decks: their memory built from the stick's own cues and grid, and
//! their edits kept for the session in [`GuestSticks`](crate::guest::GuestSticks), never in
//! the library.

use std::sync::Arc;

use analysis::BpmRange;
use engine::{DeckId, Grid, HOTCUES, Hotcue, TrackBuffer, TrackMemory};
use interop::guest::GuestTrack;
use ui::HotcueMark;

use super::memory_sync::MemoryAction;
use crate::format::safe;
use crate::guest::{HotcueMarks, plausible_bpm};

/// What a deck holding a guest track keeps of it.
#[derive(Debug, Clone)]
pub(super) struct GuestDeck {
    /// The memory the track was loaded with, as the engine applied it: the base of the
    /// session's edits until the first one is kept.
    pub loaded: TrackMemory,
    /// The hotcues' colours and names, as the window shows them.
    pub marks: Arc<HotcueMarks>,
    /// The stick was removed: the deck plays on what was decoded.
    pub gone: bool,
}

/// The memory of `track` decoded at `sample_rate` to `length_frames` frames: positions are
/// `seconds × sample_rate`. A cue before 0 or at or past the end is dropped, and so is a
/// saved loop that runs past the end; the count of dropped cues comes with it. The grid
/// comes from the track's BPM and first beat, only when it has both and the BPM is one a grid
/// can be set to; otherwise there is none, and the loader detects one (Ruling R18).
pub(super) fn guest_memory(
    track: &GuestTrack,
    sample_rate: u32,
    length_frames: usize,
) -> (TrackMemory, usize) {
    let rate = f64::from(sample_rate);
    #[expect(
        clippy::cast_precision_loss,
        reason = "track lengths are far below 2^52 frames"
    )]
    let end = length_frames as f64;
    let inside = |frames: f64| frames.is_finite() && (0.0..end).contains(&frames);
    let mut dropped = 0;
    let mut memory = TrackMemory::default();
    if let Some(seconds) = track.main_cue {
        let frames = seconds * rate;
        if inside(frames) {
            memory.cue = frames;
        } else {
            dropped += 1;
        }
    }
    for cue in &track.hotcues {
        // A pad the deck does not have is dropped like a cue past the end.
        let Some(slot) = memory.hotcues.get_mut(usize::from(cue.slot)) else {
            dropped += 1;
            continue;
        };
        let frames = cue.seconds * rate;
        // A loop with no length worth playing is a plain hotcue.
        let loop_frames = cue
            .loop_seconds
            .map(|seconds| seconds * rate)
            .filter(|l| l.is_finite() && *l > 0.0);
        if !inside(frames) || loop_frames.is_some_and(|l| frames + l > end) {
            dropped += 1;
            continue;
        }
        *slot = Some(Hotcue {
            frames,
            loop_frames,
        });
    }
    memory.grid = plausible_bpm(track.bpm)
        .zip(track.first_beat)
        .and_then(|(bpm, first)| Grid::from_bpm(bpm, first * rate, sample_rate));
    (memory, dropped)
}

/// The colours and names of `track`'s cues kept in `memory`.
pub(super) fn guest_marks(track: &GuestTrack, memory: &TrackMemory) -> HotcueMarks {
    let mut marks = HotcueMarks::default();
    for cue in &track.hotcues {
        let slot = usize::from(cue.slot);
        if slot < HOTCUES && memory.hotcues[slot].is_some() {
            marks[slot] = Some(HotcueMark::new(cue.color, cue.name.clone()));
        }
    }
    marks
}

/// `Deck 1: 2 cues of Title are past the end of the file`.
pub(super) fn dropped_text(deck: DeckId, dropped: usize, title: Option<&str>) -> String {
    let n = deck.index() + 1;
    let title = safe(title.unwrap_or("(untitled)"));
    if dropped == 1 {
        format!("Deck {n}: 1 cue of {title} is past the end of the file")
    } else {
        format!("Deck {n}: {dropped} cues of {title} are past the end of the file")
    }
}

/// A grid for a track whose app gives none, detected from the decoded `track` within
/// `range`, for the deck only.
pub(super) fn detected_grid(track: &TrackBuffer, range: BpmRange) -> Option<Grid> {
    let found = analysis::detect_beatgrid(track, range)?;
    Grid::from_bpm(found.bpm, found.first_beat_frames, track.sample_rate())
}

/// Applies the deck changes `actions` to the session's `memory` and `marks`: a deleted
/// hotcue loses its colour and name.
pub(super) fn apply(actions: &[MemoryAction], memory: &mut TrackMemory, marks: &mut HotcueMarks) {
    for &action in actions {
        match action {
            MemoryAction::SaveCue(frames) => memory.cue = frames,
            MemoryAction::SaveHotcue(slot, hotcue) => memory.hotcues[slot.index()] = Some(hotcue),
            MemoryAction::DeleteHotcue(slot) => {
                memory.hotcues[slot.index()] = None;
                marks[slot.index()] = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use actions::GridEditRequest;
    use engine::{Command, DeckCommand, DeckId, EngineController, Hotcue, HotcueSlot};
    use interop::guest::{GuestApp, GuestCue};
    use library::Library;

    use super::*;
    use crate::guest::GuestSticks;
    use crate::library_mode::LibraryMode;
    use crate::library_mode::stem_jobs::{SeparatorSource, StemsSetup};
    use crate::printer::Printer;

    const GUEST: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../interop/tests/fixtures/guest"
    );

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// The fixture stick copied to `<dir>/OJGUEST`, its mount path.
    fn fixture_stick(dir: &Path) -> PathBuf {
        let root = dunce::canonicalize(dir).unwrap().join("OJGUEST");
        copy_dir(Path::new(GUEST), &root);
        root
    }

    /// The fixture's VirtualDJ track titled `title`.
    fn vdj_track(root: &Path, title: &str) -> GuestTrack {
        let stick = interop::guest::read_stick(root, "OJGUEST");
        let source = stick
            .sources
            .into_iter()
            .find(|s| s.app == GuestApp::VirtualDj)
            .unwrap();
        source
            .tracks
            .into_iter()
            .find(|t| t.title.as_deref() == Some(title))
            .unwrap()
    }

    #[test]
    fn a_guest_track_memory_is_built_at_the_decoded_rate() {
        let dir = tempfile::tempdir().unwrap();
        let root = fixture_stick(dir.path());
        let track = vdj_track(&root, "OJ Click 126");
        let rate = 44_100;
        let (memory, dropped) = guest_memory(&track, rate, 44_100 * 600);
        assert_eq!(dropped, 0);
        let r = f64::from(rate);
        let at = |slot: usize| memory.hotcues[slot].unwrap();
        assert!((at(0).frames - 10.554_195 * r).abs() < 1.0);
        assert_eq!(at(0).loop_frames, None);
        assert!((at(1).frames - 28.60 * r).abs() < 1.0);
        assert!((at(2).frames - 46.072_268 * r).abs() < 1.0);
        // Slot 3 (pad 4) is an 8-beat loop at 126 BPM.
        assert!((at(3).frames - 38.120 * r).abs() < 1.0);
        let eight_beats = 8.0 * 60.0 / 126.0 * r;
        assert!((at(3).loop_frames.unwrap() - eight_beats).abs() < 0.001 * r);
        assert!(memory.hotcues[4..].iter().all(Option::is_none));
        // VirtualDJ gives a BPM but no first beat: no grid here, so the loader detects one.
        assert_eq!(memory.grid, None);
    }

    #[test]
    fn cues_outside_the_decoded_length_are_counted_and_dropped() {
        let cue = |slot, seconds, loop_seconds| GuestCue {
            slot,
            seconds,
            loop_seconds,
            color: None,
            name: None,
        };
        let track = GuestTrack {
            path: PathBuf::from("/s/a.wav"),
            title: Some("A".into()),
            artist: None,
            album: None,
            bpm: None,
            first_beat: None,
            main_cue: Some(-1.0),
            hotcues: vec![
                cue(0, 1.0, None),
                cue(1, -0.5, None),
                cue(2, 10.0, None),
                cue(3, 9.0, Some(2.0)),
                cue(4, 9.0, Some(1.0)),
                // A pad the deck does not have.
                cue(9, 2.0, None),
            ],
        };
        // 10 s at 1 kHz.
        let (memory, dropped) = guest_memory(&track, 1000, 10_000);
        assert_eq!(
            dropped, 5,
            "the main cue, slots 1, 2, the loop of slot 3 and slot 9"
        );
        assert_eq!(memory.cue, 0.0);
        assert_eq!(memory.hotcues[0], Some(Hotcue::point(1000.0)));
        assert_eq!(memory.hotcues[1..4], [None, None, None]);
        assert_eq!(
            memory.hotcues[4],
            Some(Hotcue {
                frames: 9000.0,
                loop_frames: Some(1000.0)
            })
        );
        assert_eq!(memory.grid, None);
    }

    #[test]
    fn a_guest_grid_needs_a_plausible_bpm_and_a_first_beat() {
        let with = |bpm: f64, first_beat: Option<f64>| GuestTrack {
            path: PathBuf::from("/s/a.wav"),
            title: Some("A".into()),
            artist: None,
            album: None,
            bpm: Some(bpm),
            first_beat,
            main_cue: None,
            hotcues: Vec::new(),
        };
        let grid = |track: &GuestTrack| guest_memory(track, 1000, 10_000).0.grid;
        // A crafted or damaged BPM, or one outside what a grid can be set to: no grid, so
        // the loader detects one.
        for bpm in [1e9, f64::INFINITY, f64::NAN, 0.0, -120.0, 39.0, 301.0] {
            assert_eq!(grid(&with(bpm, Some(0.5))), None, "bpm {bpm}");
        }
        // A BPM with no first beat (VirtualDJ), or a first beat that is no number: no grid.
        assert_eq!(grid(&with(126.0, None)), None);
        assert_eq!(grid(&with(126.0, Some(f64::NAN))), None);
        // In range with a first beat: the app's grid, at the decoded rate.
        let kept = grid(&with(126.0, Some(0.5))).expect("a grid");
        assert!((kept.beat_period_frames - 60.0 * 1000.0 / 126.0).abs() < 1e-6);
        assert!(grid(&with(40.0, Some(0.0))).is_some());
        assert!(grid(&with(300.0, Some(0.0))).is_some());
    }

    type Events = Arc<Mutex<Vec<String>>>;

    struct Rig {
        _dir: tempfile::TempDir,
        root: PathBuf,
        mode: LibraryMode,
        engine: EngineController,
        renderer: engine::EngineRenderer,
        printer: Printer,
        events: Events,
    }

    /// Library mode on a new library (with stems per `stems`), the fixture stick copied to
    /// a temp mount and plugged in.
    fn rig(stems: Option<StemsSetup>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = fixture_stick(dir.path());
        let mode =
            LibraryMode::start_with(Some(&dir.path().join("library.sqlite3")), stems).unwrap();
        let (engine, renderer) = engine::engine(engine::EngineConfig {
            sample_rate: 44_100,
            max_block_frames: 512,
        });
        let (printer, events) = Printer::capturing();
        let rig = Rig {
            _dir: dir,
            root,
            mode,
            engine,
            renderer,
            printer,
            events,
        };
        plug(&rig);
        rig
    }

    /// Plugs the rig's stick in, as the stick reader would.
    fn plug(rig: &Rig) {
        let stick = interop::guest::read_stick(&rig.root, "OJGUEST");
        let plug = rig.mode.sticks.expect(&stick.root);
        assert!(rig.mode.sticks.add(&stick, plug));
    }

    /// The id of the plugged-in VirtualDJ track titled `title`.
    fn vdj_id(sticks: &GuestSticks, title: &str) -> i64 {
        app_id(sticks, GuestApp::VirtualDj, title)
    }

    /// The id of the plugged-in track titled `title` of `app`'s branch.
    fn app_id(sticks: &GuestSticks, app: GuestApp, title: &str) -> i64 {
        let all = sticks.sticks();
        let source = all[0].sources.iter().find(|s| s.app == app).unwrap();
        *source
            .tracks
            .iter()
            .find(|&&id| sticks.track(id).unwrap().row.title.as_deref() == Some(title))
            .unwrap()
    }

    fn deck(n: usize) -> DeckId {
        DeckId::new(n).unwrap()
    }

    fn render(renderer: &mut engine::EngineRenderer) {
        let (mut master, mut headphones) = (vec![0.0; 1024], vec![0.0; 1024]);
        renderer.render(&mut master, &mut headphones);
    }

    /// Renders and polls until `done` holds, for at most 10 s.
    fn run_until(rig: &mut Rig, mut done: impl FnMut(&LibraryMode, &EngineController) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            render(&mut rig.renderer);
            rig.mode.poll(Instant::now(), &mut rig.engine, &rig.printer);
            if done(&rig.mode, &rig.engine) {
                return;
            }
            assert!(Instant::now() < deadline, "not done within 10 s");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Loads `id` on deck `n` (0-based) and waits until the engine plays it.
    fn load(rig: &mut Rig, n: usize, id: i64) {
        let before = rig.mode.decks[n].as_ref().map_or(0, |t| t.load_seq);
        rig.mode.load_track(deck(n), id).unwrap();
        run_until(rig, |mode, _| {
            mode.decks[n]
                .as_ref()
                .is_some_and(|t| t.row.id == id && t.load_seq > before && t.memory.load_applied())
        });
    }

    /// Waits until an event `matches`, for at most 10 s, polling meanwhile.
    fn wait_event(rig: &mut Rig, matches: impl Fn(&str) -> bool) {
        let events = Arc::clone(&rig.events);
        run_until(rig, |_, _| {
            events.lock().unwrap().iter().any(|e| matches(e))
        });
    }

    /// Every library table with its row count.
    fn table_rows(lib: &Library) -> Vec<(String, i64)> {
        let reader = lib.reader().unwrap();
        let conn = reader.connection();
        let names: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        names
            .into_iter()
            .map(|name| {
                let count = conn
                    .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                (name, count)
            })
            .collect()
    }

    fn rows_of(lib: &Library, table: &str) -> i64 {
        table_rows(lib)
            .into_iter()
            .find(|(name, _)| name == table)
            .unwrap()
            .1
    }

    fn flush(mode: &LibraryMode) {
        mode.writer
            .submit_wait(library::WriteOp::Batch(Vec::new()))
            .unwrap();
    }

    fn assert_library_untouched(mode: &LibraryMode) {
        flush(mode);
        for table in ["tracks", "cues", "hotcues", "analysis"] {
            assert_eq!(rows_of(&mode.lib, table), 0, "{table}");
        }
        assert_eq!(mode.lib.failed_write_count(), 0);
    }

    /// No warning but the decoder's about the fixture's cut MP3s.
    fn no_warnings(events: &Events) {
        let events = events.lock().unwrap();
        assert!(
            events.iter().all(|e| !e.contains("warning")
                || e.ends_with("may be incomplete (decoding stopped early)")),
            "{events:?}"
        );
    }

    #[test]
    fn a_guest_track_loads_with_its_cues() {
        let mut rig = rig(None);
        let id = vdj_id(&rig.mode.sticks, "OJ Click 126");
        load(&mut rig, 0, id);
        // The fixture's audio is cut short: every cue of the track is past its end.
        wait_event(&mut rig, |e| {
            e == "Deck 1: 4 cues of OJ Click 126 are past the end of the file"
        });
        assert_library_untouched(&rig.mode);
        no_warnings(&rig.events);
    }

    #[test]
    fn a_serato_guest_track_loads_with_its_tempo_map_grid() {
        let mut rig = rig(None);
        let id = app_id(&rig.mode.sticks, GuestApp::Serato, "OJ Click 126");
        load(&mut rig, 0, id);
        // The app's grid, at the decoded rate: the tempo map's BPM and first beat.
        let track = rig.mode.sticks.track(id).unwrap().track;
        let rate = f64::from(rig.engine.deck(deck(0)).track_sample_rate);
        let grid = rig.engine.deck(deck(0)).grid.unwrap();
        assert!((grid.beat_period_frames - 60.0 * rate / 126.000_728).abs() < 0.01);
        let first = track.first_beat.unwrap() * rate;
        assert!((grid.first_beat_frames - first).abs() < 0.01);
        assert_library_untouched(&rig.mode);
    }

    #[test]
    fn cues_past_the_end_are_dropped_with_a_message() {
        let mut rig = rig(None);
        let id = vdj_id(&rig.mode.sticks, "OJ Click 120");
        load(&mut rig, 0, id);
        wait_event(&mut rig, |e| {
            e == "Deck 1: 3 cues of OJ Click 120 are past the end of the file"
        });
        assert_eq!(rig.engine.deck(deck(0)).hotcues, [None; engine::HOTCUES]);
        assert_library_untouched(&rig.mode);
    }

    #[test]
    fn a_guest_hotcue_edit_stays_for_the_session_only() {
        let mut rig = rig(None);
        let id = vdj_id(&rig.mode.sticks, "OJ Click 120");
        load(&mut rig, 0, id);
        let slot5 = HotcueSlot::new(5).unwrap();
        rig.engine
            .send(Command::Deck(
                deck(0),
                DeckCommand::SetHotcue(slot5, Some(Hotcue::point(1000.0))),
            ))
            .unwrap();
        run_until(&mut rig, |mode, _| {
            mode.sticks
                .hotcues(id)
                .is_some_and(|m| m.hotcues[4] == Some(Hotcue::point(1000.0)))
        });
        // Loaded again (on another deck too): the edit is there.
        load(&mut rig, 0, id);
        load(&mut rig, 1, id);
        for n in [0, 1] {
            assert_eq!(
                rig.engine.deck(deck(n)).hotcues[4],
                Some(Hotcue::point(1000.0))
            );
        }
        // Removed and plugged in again: the stick's tracks have new ids, without the edit.
        assert_eq!(
            rig.mode.sticks.remove(&rig.root).as_deref(),
            Some("OJGUEST")
        );
        plug(&rig);
        let again = vdj_id(&rig.mode.sticks, "OJ Click 120");
        assert_ne!(again, id);
        load(&mut rig, 0, again);
        assert_eq!(rig.engine.deck(deck(0)).hotcues[4], None);
        assert_library_untouched(&rig.mode);
        no_warnings(&rig.events);
    }

    #[test]
    fn a_guest_deck_keeps_playing_when_the_stick_goes() {
        let mut rig = rig(None);
        let id = vdj_id(&rig.mode.sticks, "OJ Click 120");
        load(&mut rig, 0, id);
        rig.engine
            .send(Command::Deck(deck(0), DeckCommand::Play))
            .unwrap();
        run_until(&mut rig, |_, engine| engine.deck(deck(0)).playing);
        rig.mode.sticks.remove(&rig.root);
        run_until(&mut rig, |mode, _| {
            mode.decks[0].as_ref().and_then(|t| t.row.title.as_deref())
                == Some("OJ Click 120 (stick removed)")
        });
        assert!(rig.engine.deck(deck(0)).playing);
        // The gone track cannot be loaded again.
        rig.mode.load_track(deck(1), id).unwrap();
        wait_event(&mut rig, |e| e == "Stick OJGUEST is no longer plugged in");
        assert!(rig.mode.decks[1].is_none());
    }

    #[test]
    fn nothing_reaches_the_library_after_a_guest_set() {
        let stems = StemsSetup {
            budget_bytes: 1_000_000_000,
            kicks_hz: engine::DEFAULT_KICKS_HZ,
            separator: SeparatorSource::Missing,
        };
        let mut rig = rig(Some(stems));
        flush(&rig.mode);
        let before = table_rows(&rig.mode.lib);
        // Serato's 120 has a grid (its tempo map) to shift.
        let id = app_id(&rig.mode.sticks, GuestApp::Serato, "OJ Click 120");
        load(&mut rig, 0, id);
        // Cue point, a hotcue and its label.
        let slot2 = HotcueSlot::new(2).unwrap();
        for command in [
            DeckCommand::SetCue(500.0),
            DeckCommand::SetHotcue(slot2, Some(Hotcue::point(2000.0))),
        ] {
            rig.engine.send(Command::Deck(deck(0), command)).unwrap();
        }
        run_until(&mut rig, |mode, _| {
            mode.sticks
                .hotcues(id)
                .is_some_and(|m| m.cue == 500.0 && m.hotcues[1].is_some())
        });
        rig.mode
            .set_hotcue_label(deck(0), id, slot2, Some("Drop".into()), Some(0xff0000))
            .unwrap();
        // Shown from the deck's memory, and kept for the session.
        let drop = Some(ui::HotcueMark::new(Some(0xff0000), Some("Drop".into())));
        let snap = rig.engine.deck(deck(0));
        let view = rig.mode.deck_view(deck(0), &snap).unwrap();
        assert_eq!(view.hotcue_marks.unwrap()[1], drop);
        assert_eq!(rig.mode.sticks.marks(id).unwrap()[1], drop);
        // A grid shift, in memory.
        let period = rig.engine.deck(deck(0)).grid.unwrap().beat_period_frames;
        let first = rig.engine.deck(deck(0)).grid.unwrap().first_beat_frames;
        rig.mode.handle_request(
            actions::AppRequest::Grid {
                deck: deck(0),
                edit: GridEditRequest::Shift { millis: 10 },
            },
            &rig.engine,
            &rig.printer,
        );
        run_until(&mut rig, |_, engine| {
            engine
                .deck(deck(0))
                .grid
                .is_some_and(|g| g.first_beat_frames != first)
        });
        let shifted = rig.engine.deck(deck(0)).grid.unwrap();
        assert!((shifted.beat_period_frames - period).abs() < 1e-9);
        assert!((shifted.first_beat_frames - (first + 441.0).rem_euclid(period)).abs() < 1e-6);
        // Stems and covers: none for a guest.
        assert!(rig.mode.remove_cover(id).is_err());
        assert_eq!(rig.mode.cover_file(id), None);
        for _ in 0..20 {
            run_until(&mut rig, |_, _| true);
        }
        // No stems asked for.
        let track = rig.mode.decks[0].as_ref().unwrap();
        assert!(matches!(track.stems, super::super::DeckStems::Done));
        assert!(track.buffer.is_none());
        let stems = rig.mode.stems.as_ref().unwrap();
        assert!(stems.idle());
        assert!(stems.queue_progress().is_none_or(|(_, total)| total == 0));
        flush(&rig.mode);
        assert_eq!(table_rows(&rig.mode.lib), before);
        assert_eq!(rig.mode.lib.failed_write_count(), 0);
        no_warnings(&rig.events);
    }

    /// Kick-like clicks at `bpm` for `secs`, as a 16-bit stereo WAV at 44.1 kHz.
    fn write_clicks(path: &Path, bpm: f64, secs: f64) {
        let sr = 44_100.0;
        let n = (secs * sr) as usize;
        let mut mono = vec![0.0f64; n];
        let period = 60.0 / bpm * sr;
        let burst = (0.25 * sr) as usize;
        let mut beat = 0.0;
        loop {
            let start = (0.25 * sr + beat * period).round() as usize;
            if start >= n {
                break;
            }
            for k in 0..burst.min(n - start) {
                let t = k as f64 / sr;
                mono[start + k] += (std::f64::consts::TAU * 60.0 * t).sin() * (-t / 0.05).exp();
            }
            beat += 1.0;
        }
        let data_len = u32::try_from(n * 4).unwrap();
        let mut bytes = Vec::with_capacity(44 + n * 4);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for v in mono {
            let s = (v.clamp(-1.0, 1.0) * 0.8 * f64::from(i16::MAX)) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_guest_track_without_a_grid_gets_one_in_memory() {
        let mut rig = rig(None);
        // A second stick whose app gives no BPM.
        let other = rig.root.with_file_name("NOGRID");
        std::fs::create_dir_all(&other).unwrap();
        let file = other.join("clicks.wav");
        write_clicks(&file, 120.0, 20.0);
        let stick = interop::guest::GuestStick {
            root: other.clone(),
            name: "NOGRID".into(),
            sources: vec![interop::guest::GuestSource {
                app: GuestApp::Serato,
                tracks: vec![GuestTrack {
                    path: file,
                    title: Some("Clicks".into()),
                    artist: None,
                    album: None,
                    bpm: None,
                    first_beat: None,
                    main_cue: None,
                    hotcues: Vec::new(),
                }],
                playlists: Vec::new(),
                problem: None,
            }],
        };
        let plug = rig.mode.sticks.expect(&other);
        assert!(rig.mode.sticks.add(&stick, plug));
        let id = rig
            .mode
            .sticks
            .sticks()
            .iter()
            .find(|s| s.root == other)
            .unwrap()
            .sources[0]
            .tracks[0];
        load(&mut rig, 0, id);
        let grid = rig.engine.deck(deck(0)).grid.expect("a detected grid");
        let bpm = 60.0 * 44_100.0 / grid.beat_period_frames;
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
        assert_library_untouched(&rig.mode);
        no_warnings(&rig.events);
    }

    #[test]
    fn a_guest_load_landing_after_its_stick_went_says_so() {
        let mut rig = rig(None);
        let id = vdj_id(&rig.mode.sticks, "OJ Click 120");
        let track = rig.mode.sticks.track(id).unwrap();
        // Pulled while the track decodes: the control loop sees it before the load lands.
        rig.mode.sticks.remove(&rig.root);
        run_until(&mut rig, |_, _| true);
        let seq = rig.mode.next_seq(deck(0));
        let loaded = super::super::worker::Loaded {
            deck: deck(0),
            seq,
            row: track.row,
            path: track.path,
            buffer: Arc::new(engine::TrackBuffer::new(44_100, vec![0.0; 2 * 4410]).unwrap()),
            memory: TrackMemory::default(),
            hotcues_read: true,
            refuse_if_playing: false,
            guest: Some(super::super::worker::GuestLoaded {
                marks: Default::default(),
                dropped: 0,
            }),
        };
        rig.mode.load(loaded, &mut rig.engine, &rig.printer);
        run_until(&mut rig, |_, _| true);
        let title = rig.mode.decks[0].as_ref().unwrap().row.title.clone();
        assert_eq!(title.as_deref(), Some("OJ Click 120 (stick removed)"));
    }

    /// Asks for `edits` on deck 1 in one round, then waits until its grid is `expected`
    /// (period, first beat, from the rate), within a frame.
    fn edit_grid(rig: &mut Rig, edits: &[GridEditRequest], expected: impl Fn(f64) -> (f64, f64)) {
        let rate = f64::from(rig.engine.deck(deck(0)).track_sample_rate);
        for &edit in edits {
            rig.mode.handle_request(
                actions::AppRequest::Grid {
                    deck: deck(0),
                    edit,
                },
                &rig.engine,
                &rig.printer,
            );
        }
        let (period, first) = expected(rate);
        let close = |g: Option<engine::Grid>| {
            g.is_some_and(|g| {
                (g.beat_period_frames - period).abs() < 1.0
                    && (g.first_beat_frames - first).abs() < 1.0
            })
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            run_until(rig, |_, _| true);
            if close(rig.engine.deck(deck(0)).grid) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "grid {:?}, wanted period {period} first {first}",
                rig.engine.deck(deck(0)).grid
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Loads Serato's 120 (its tempo map's grid: 0.4994 s at 119.996 BPM) on deck 1, and
    /// gives its id and grid.
    fn load_serato_120(rig: &mut Rig) -> (i64, engine::Grid) {
        let id = app_id(&rig.mode.sticks, GuestApp::Serato, "OJ Click 120");
        load(rig, 0, id);
        (
            id,
            rig.engine.deck(deck(0)).grid.expect("the tempo map's grid"),
        )
    }

    #[test]
    fn guest_grid_edits_in_a_row_all_apply_in_order() {
        let mut rig = rig(None);
        let (id, start) = load_serato_120(&mut rig);
        let (period, first) = (start.beat_period_frames, start.first_beat_frames);
        let shift = GridEditRequest::Shift { millis: 10 };
        // Two shifts: 20 ms.
        edit_grid(&mut rig, &[shift, shift], |r| {
            (period, (first + 0.02 * r).rem_euclid(period))
        });
        // Double then shift: the doubled grid, shifted.
        edit_grid(&mut rig, &[GridEditRequest::Double, shift], |r| {
            let half = period / 2.0;
            (
                half,
                ((first + 0.02 * r).rem_euclid(period) + 0.01 * r).rem_euclid(half),
            )
        });
        let kept = rig.mode.sticks.hotcues(id).unwrap().grid;
        assert_eq!(kept, rig.engine.deck(deck(0)).grid);
    }

    #[test]
    fn a_guest_grid_edit_waits_for_room_in_a_full_engine_queue() {
        let mut rig = rig(None);
        let (id, start) = load_serato_120(&mut rig);
        let rate = f64::from(rig.engine.deck(deck(0)).track_sample_rate);
        let want = (start.first_beat_frames + 0.01 * rate).rem_euclid(start.beat_period_frames);
        while rig
            .engine
            .send(Command::Deck(deck(0), DeckCommand::Pause))
            .is_ok()
        {}
        rig.mode.handle_request(
            actions::AppRequest::Grid {
                deck: deck(0),
                edit: GridEditRequest::Shift { millis: 10 },
            },
            &rig.engine,
            &rig.printer,
        );
        // Not rendered: the queue stays full while the edit comes back.
        let deadline = Instant::now() + Duration::from_secs(10);
        let events = Arc::clone(&rig.events);
        while !events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.starts_with("Deck 1 grid:"))
        {
            assert!(Instant::now() < deadline, "no grid event");
            rig.mode.poll(Instant::now(), &mut rig.engine, &rig.printer);
            std::thread::sleep(Duration::from_millis(2));
        }
        // Kept for the session at once, and on the deck once there is room.
        let kept = rig.mode.sticks.hotcues(id).and_then(|m| m.grid);
        assert!(
            kept.is_some_and(|g| (g.first_beat_frames - want).abs() < 1.0),
            "{kept:?}"
        );
        run_until(&mut rig, |_, engine| {
            engine
                .deck(deck(0))
                .grid
                .is_some_and(|g| (g.first_beat_frames - want).abs() < 1.0)
        });
    }
}
