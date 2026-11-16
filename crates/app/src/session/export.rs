//! The export to a USB stick in the window: the window's [`ExportStep`]s move it along and
//! the frame shows where it is ([`ExportView`]). Listing the sticks, checking one and
//! writing the export each run on a thread of their own: the control loop never waits.
//!
//! The export only reads the library, so the decks may hold tracks meanwhile.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use interop::export::REKORDBOX_STEPS;
use interop::{ExportControl, ExportPlan, ExportProgress, ExportReport, Manifest};
use library::Library;
use ui::{ExportChoice, ExportStep, ExportSummary, ExportView, JobView, PlaylistLabel, StickLabel};

use super::jobs::{self, ExportFacts, Pace};

use crate::format::{megabytes, safe};
use crate::library_cmd::export_summary;
use crate::sticks::{Mounts, Stick, list_sticks};

/// The window's export to a USB stick.
pub(super) struct Export {
    lib: Arc<Library>,
    mounts: Mounts,
    state: State,
    view: ExportView,
    /// Measures the writing's speed for the footer's time left; only the control thread
    /// touches it.
    pace: Mutex<Pace>,
    /// News for the event line, taken by [`Export::take_events`].
    events: Vec<String>,
}

/// What the export is doing, besides what it shows.
enum State {
    /// Nothing: not started, put away, or finished.
    Rest,
    Listing(Receiver<Result<Listed, String>>),
    /// The choice shows.
    Choosing(Listed),
    Checking(Listed, Receiver<Result<Checked, String>>),
    /// The summary shows.
    Checked(Listed, Box<Checked>),
    /// Writing to a stick.
    Writing(Writing),
}

/// An export being written on its thread.
struct Writing {
    rx: Receiver<Written>,
    /// The stick's name.
    stick: String,
    /// What the footer's pause and stop say to the thread, and what it says back.
    run: Arc<Run>,
    /// What each track is called, in the order they are copied.
    names: Vec<String>,
}

/// What the window and the writing thread tell each other, with no lock: the thread reads
/// the flags between two files and the control loop reads the counts for the footer.
#[derive(Default)]
struct Run {
    /// The footer paused the export: the thread waits after the file it is on.
    paused: AtomicBool,
    /// The footer stopped it: the thread stops after the file it is on.
    stop: AtomicBool,
    /// The thread is waiting for a resume, copying nothing.
    waiting: AtomicBool,
    /// Steps the thread finished: one per track, then one for the library files.
    done: AtomicUsize,
}

impl Run {
    /// Called by the export between two files: waits while paused (a stop ends the wait)
    /// and says whether to go on.
    fn control(&self, progress: ExportProgress) -> ExportControl {
        self.done.store(progress.done, Ordering::Relaxed);
        loop {
            if self.stop.load(Ordering::Relaxed) {
                self.waiting.store(false, Ordering::Relaxed);
                return ExportControl::Stop;
            }
            if !self.paused.load(Ordering::Relaxed) {
                self.waiting.store(false, Ordering::Relaxed);
                return ExportControl::Go;
            }
            self.waiting.store(true, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The sticks found, and the choice made of them for the window.
struct Listed {
    sticks: Vec<Stick>,
    choice: ExportChoice,
}

/// A checked export: its plan, and what the summary shows.
struct Checked {
    plan: ExportPlan,
    summary: ExportSummary,
}

/// What the writing thread sends: progress, then the result.
enum Written {
    Progress(ExportProgress),
    Done(Result<ExportReport, String>),
}

impl Export {
    /// An export from `lib` to the sticks mounted in `mounts`.
    pub fn new(lib: Arc<Library>, mounts: Mounts) -> Self {
        Self {
            lib,
            mounts,
            state: State::Rest,
            view: ExportView::Idle,
            pace: Mutex::new(Pace::new()),
            events: Vec::new(),
        }
    }

    /// The footer's pill while the export writes.
    pub fn job(&self, now: Instant) -> Option<JobView> {
        let State::Writing(w) = &self.state else {
            return None;
        };
        let files = w.names.len();
        let done = w.run.done.load(Ordering::Relaxed).min(files);
        let paused = w.run.paused.load(Ordering::Relaxed);
        let eta = {
            let mut pace = self.pace.lock().unwrap_or_else(PoisonError::into_inner);
            if paused {
                pace.pause();
            } else {
                pace.sample(now, done, files);
            }
            pace.eta(now)
        };
        Some(jobs::export_job(ExportFacts {
            stick: w.stick.clone(),
            done,
            total: files,
            paused,
            waiting: w.run.waiting.load(Ordering::Relaxed),
            current: w.names.get(done).cloned(),
            eta,
        }))
    }

    /// The footer's pause and resume: the export waits after the file it is on.
    pub fn pause(&mut self, paused: bool) {
        if let State::Writing(w) = &self.state {
            w.run.paused.store(paused, Ordering::Relaxed);
        }
    }

    /// The footer's stop: the export stops after the file it is on, before any library
    /// file is written.
    pub fn stop(&mut self) {
        if let State::Writing(w) = &self.state {
            w.run.stop.store(true, Ordering::Relaxed);
        }
    }

    /// What the export has to say to the event line since the last call.
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// What the window shows.
    pub fn view(&self) -> &ExportView {
        &self.view
    }

    /// Takes the window's `step`. A step that does not fit where the export is (a write
    /// with no summary, a stick not listed, anything while it writes) is ignored.
    pub fn step(&mut self, step: ExportStep) {
        if matches!(self.state, State::Writing { .. }) {
            return;
        }
        match step {
            ExportStep::Open => self.list(),
            ExportStep::Check { stick, playlists } => self.check(stick, playlists),
            ExportStep::Write => self.write(),
            ExportStep::Back => {
                if matches!(self.state, State::Checked(..))
                    && let State::Checked(listed, _) =
                        std::mem::replace(&mut self.state, State::Rest)
                {
                    self.choose(listed);
                }
            }
            ExportStep::Close => {
                if matches!(
                    self.view,
                    ExportView::Choose(_) | ExportView::Done(_) | ExportView::Failed(_)
                ) {
                    self.state = State::Rest;
                    self.view = ExportView::Idle;
                }
            }
        }
    }

    /// Takes the threads' news, if any came; `true` when the view changed.
    pub fn poll(&mut self) -> bool {
        let before = self.view.clone();
        match std::mem::replace(&mut self.state, State::Rest) {
            State::Listing(rx) => match rx.try_recv() {
                Err(TryRecvError::Empty) => self.state = State::Listing(rx),
                Ok(Ok(listed)) => self.choose(listed),
                Ok(Err(why)) => self.fail(why),
                Err(TryRecvError::Disconnected) => self.fail(STOPPED.to_owned()),
            },
            State::Checking(listed, rx) => match rx.try_recv() {
                Err(TryRecvError::Empty) => self.state = State::Checking(listed, rx),
                Ok(Ok(checked)) => {
                    self.view = ExportView::Summary(checked.summary.clone());
                    self.state = State::Checked(listed, Box::new(checked));
                }
                Ok(Err(why)) => self.fail(why),
                Err(TryRecvError::Disconnected) => self.fail(STOPPED.to_owned()),
            },
            State::Writing(w) => self.take_written(w),
            other => self.state = other,
        }
        self.view != before
    }

    /// Takes what the writing thread sent since the last poll.
    fn take_written(&mut self, w: Writing) {
        loop {
            match w.rx.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.state = State::Writing(w);
                    return;
                }
                Ok(Written::Progress(p)) => {
                    self.view = ExportView::Writing {
                        done: p.done,
                        total: p.total,
                    };
                }
                Ok(Written::Done(result)) => {
                    match result {
                        Ok(report) if report.stopped => {
                            let text = stopped_text(&report, w.names.len());
                            self.events.push(text.clone());
                            self.view = ExportView::Done(text);
                        }
                        Ok(report) => self.view = ExportView::Done(done_text(&report, &w.stick)),
                        Err(why) => self.view = ExportView::Failed(why),
                    }
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.fail(STOPPED.to_owned());
                    return;
                }
            }
        }
    }

    /// Lists the sticks and the playlists on a thread.
    fn list(&mut self) {
        let (lib, mounts) = (Arc::clone(&self.lib), self.mounts.clone());
        let (tx, rx) = channel();
        if self.spawn(move || {
            // A listing put away has no one to send to.
            let _ = tx.send(listed(&lib, &mounts));
        }) {
            self.state = State::Listing(rx);
            self.view = ExportView::Listing;
        }
    }

    /// Plans the export of `playlists` to the listed stick `stick` on a thread.
    fn check(&mut self, stick: usize, playlists: Vec<i64>) {
        let State::Choosing(listed) = &self.state else {
            return;
        };
        let Some(chosen) = listed.sticks.get(stick).cloned() else {
            return;
        };
        if playlists.is_empty() {
            return;
        }
        let State::Choosing(listed) = std::mem::replace(&mut self.state, State::Rest) else {
            return;
        };
        let lib = Arc::clone(&self.lib);
        let (tx, rx) = channel();
        if self.spawn(move || {
            let _ = tx.send(checked(&lib, &chosen, &playlists));
        }) {
            self.state = State::Checking(listed, rx);
            self.view = ExportView::Checking;
        }
    }

    /// Writes the checked export on a thread, sending its progress.
    fn write(&mut self) {
        let State::Checked(_, checked) = &self.state else {
            return;
        };
        if checked.summary.refusal().is_some() {
            return;
        }
        let State::Checked(_, checked) = std::mem::replace(&mut self.state, State::Rest) else {
            return;
        };
        let Checked { plan, summary } = *checked;
        let total = plan.tracks.len() + 1;
        let names = plan
            .tracks
            .iter()
            .map(|t| {
                let track = &t.track;
                safe(match &track.artist {
                    Some(artist) => format!("{artist} – {}", track.title),
                    None => track.title.clone(),
                })
            })
            .collect();
        let run = Arc::new(Run::default());
        let (tx, rx) = channel();
        let thread_run = Arc::clone(&run);
        if self.spawn(move || {
            let control = |p: ExportProgress| {
                let _ = tx.send(Written::Progress(p));
                thread_run.control(p)
            };
            let result = interop::run(&plan, &control).map_err(|e| safe(format!("{e}")));
            let _ = tx.send(Written::Done(result));
        }) {
            *self.pace.lock().unwrap_or_else(PoisonError::into_inner) = Pace::new();
            self.state = State::Writing(Writing {
                rx,
                stick: summary.stick,
                run,
                names,
            });
            self.view = ExportView::Writing { done: 0, total };
        }
    }

    /// Shows the choice from `listed`.
    fn choose(&mut self, listed: Listed) {
        self.view = ExportView::Choose(listed.choice.clone());
        self.state = State::Choosing(listed);
    }

    /// Runs `work` on a thread; `false`, with the export failed, when none can start.
    fn spawn(&mut self, work: impl FnOnce() + Send + 'static) -> bool {
        match std::thread::Builder::new()
            .name("usb-export".to_owned())
            .spawn(work)
        {
            Ok(_) => true,
            Err(e) => {
                self.fail(format!("cannot start the export: {e}"));
                false
            }
        }
    }

    /// Stops with `why`.
    fn fail(&mut self, why: String) {
        self.state = State::Rest;
        self.view = ExportView::Failed(why);
    }
}

/// What the window shows when an export's thread died.
const STOPPED: &str = "The export stopped unexpectedly.";

/// The sticks mounted in `mounts` and the playlists of `lib`, with each stick's playlists
/// from an earlier export.
fn listed(lib: &Library, mounts: &Mounts) -> Result<Listed, String> {
    let fail = |e: anyhow::Error| safe(format!("{e:#}"));
    let reader = lib.reader().map_err(|e| fail(e.into()))?;
    let sticks = list_sticks(&reader, mounts).map_err(fail)?;
    let playlists = reader.playlists().map_err(|e| fail(e.into()))?;
    let labels = sticks
        .iter()
        .map(|stick| {
            // A damaged manifest ticks nothing; the check then says why.
            let on_stick = match Manifest::read(&stick.root) {
                Ok(Some(manifest)) => manifest.playlists,
                Ok(None) | Err(_) => Vec::new(),
            };
            StickLabel {
                name: safe(&stick.name),
                free: free_text(stick.free_bytes),
                exported: playlists
                    .iter()
                    .filter(|p| on_stick.contains(&p.name))
                    .map(|p| p.id)
                    .collect(),
            }
        })
        .collect();
    let choice = ExportChoice {
        sticks: labels,
        playlists: playlists
            .iter()
            .map(|p| PlaylistLabel {
                id: p.id,
                name: safe(&p.name),
                tracks: p.track_count,
            })
            .collect(),
    };
    Ok(Listed { sticks, choice })
}

/// Plans exporting `playlists` from `lib` to `stick`; writes nothing.
fn checked(lib: &Library, stick: &Stick, playlists: &[i64]) -> Result<Checked, String> {
    let fail = |e: interop::ExportError| safe(format!("{e}"));
    let reader = lib
        .reader()
        .map_err(|e| fail(interop::ExportError::Library(e)))?;
    let set = interop::gather(&reader, playlists).map_err(fail)?;
    let folders = reader
        .folders()
        .map_err(|e| fail(interop::ExportError::Library(e)))?;
    let plan = interop::plan_export(set, &stick.root, &folders).map_err(fail)?;
    let free = free_text(plan.free_bytes);
    let summary = export_summary(safe(&stick.name), &plan, free);
    Ok(Checked { plan, summary })
}

/// Free space as the window shows it: megabytes, or "?" when unknown.
fn free_text(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "?".to_owned(), megabytes)
}

/// What the window and the event line say of an export that was stopped after `files` tracks
/// were planned.
fn stopped_text(report: &ExportReport, files: usize) -> String {
    format!(
        "Export stopped: {} of {files} files copied; the stick keeps its previous playlists",
        report.copied + report.kept + report.retagged
    )
}

/// What the window shows once `report` is written to the stick called `stick`.
fn done_text(report: &ExportReport, stick: &str) -> String {
    let playlists = if report.playlists == 1 {
        "playlist"
    } else {
        "playlists"
    };
    let mut text = format!(
        "Exported {} {playlists} to {stick}: {} copied, {} kept.\n\
         To open them in Rekordbox (also in IMPORT TO REKORDBOX.txt on the stick):\n{REKORDBOX_STEPS}",
        report.playlists, report.copied, report.kept
    );
    for note in &report.notes {
        text.push('\n');
        text.push_str(&safe(note));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::{Duration, Instant};

    use library::{WriteOp, WriteOutcome};
    use ui::UiCommand;

    use super::super::commands;
    use super::super::frame::FrameParts;
    use super::*;
    use crate::config::Config;

    const STEREO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../decode/tests/fixtures/sine440.wav"
    );

    /// A library of one scanned track in the playlist "Mix", a temp folder standing for
    /// `/Volumes` with one stick in it, and the session's parts exporting to it.
    struct Setup {
        _dir: tempfile::TempDir,
        /// The library's one folder.
        music: PathBuf,
        /// The one stick, `<temp>/volumes/STICK`.
        stick: PathBuf,
        /// The playlist "Mix".
        mix: i64,
        parts: FrameParts,
        window: Sender<UiCommand>,
        commands: Receiver<UiCommand>,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        // Library folders are stored canonical (/private/var on macOS).
        let root = dunce::canonicalize(dir.path()).unwrap();
        let music = root.join("music");
        std::fs::create_dir_all(&music).unwrap();
        std::fs::copy(STEREO, music.join("a.wav")).unwrap();
        let lib = Arc::new(Library::open(&root.join("library.sqlite3")).unwrap());
        library::add_folder(&lib, &music).unwrap();
        library::scan_folder(&lib, &music, &mut |_| {}).unwrap();
        let mix = match lib
            .writer()
            .submit_wait(WriteOp::CreatePlaylist { name: "Mix".into() })
            .unwrap()
        {
            WriteOutcome::Created { id } => id,
            other => panic!("{other:?}"),
        };
        let track = id_of(&lib, &music.join("a.wav"));
        lib.writer()
            .submit_wait(WriteOp::AddToPlaylist {
                playlist: mix,
                track,
                at: None,
            })
            .unwrap();
        let volumes = root.join("volumes");
        let stick = volumes.join("STICK");
        std::fs::create_dir_all(&stick).unwrap();
        let mut parts = FrameParts::new(&Config::default());
        // The temp folder stands for `/Volumes`: no real drive is listed.
        let mounts = Mounts {
            dirs: vec![volumes],
            drive_letters: false,
        };
        parts.export = Some(Export::new(lib, mounts));
        let (window, commands) = channel();
        Setup {
            _dir: dir,
            music,
            stick,
            mix,
            parts,
            window,
            commands,
        }
    }

    fn id_of(lib: &Library, path: &Path) -> i64 {
        let reader = lib.reader().unwrap();
        (1..10)
            .find(|&id| reader.track(id).unwrap().is_some_and(|t| t.path == path))
            .expect("the track is in the library")
    }

    impl Setup {
        /// Sends `step` and applies it as the session does, with no engine running.
        fn send(&mut self, step: ExportStep) -> ExportView {
            self.window.send(UiCommand::Export(step)).unwrap();
            commands::drain(&self.commands, &mut self.parts, None);
            self.view()
        }

        fn view(&self) -> ExportView {
            self.parts.idle().export
        }

        /// The view once the thread's work is done.
        fn settled(&mut self) -> ExportView {
            let began = Instant::now();
            loop {
                commands::drain(&self.commands, &mut self.parts, None);
                let view = self.view();
                if !matches!(
                    view,
                    ExportView::Listing | ExportView::Checking | ExportView::Writing { .. }
                ) {
                    return view;
                }
                assert!(
                    began.elapsed() < Duration::from_secs(10),
                    "the export never finished"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn manifest(&self) -> PathBuf {
            self.stick.join("Open Jockey").join("export.toml")
        }
    }

    /// Adds `names` as copies of the first track to the library and to "Mix", and checks
    /// the export of "Mix" to the stick, ready to write.
    fn with_tracks_ready(s: &mut Setup, names: &[&str]) {
        let music = s.music.clone();
        for name in names {
            std::fs::copy(STEREO, music.join(name)).unwrap();
        }
        let lib = Arc::clone(&s.parts.export.as_ref().unwrap().lib);
        library::scan_folder(&lib, &music, &mut |_| {}).unwrap();
        for name in names {
            let track = id_of(&lib, &music.join(name));
            lib.writer()
                .submit_wait(WriteOp::AddToPlaylist {
                    playlist: s.mix,
                    track,
                    at: None,
                })
                .unwrap();
        }
        s.send(ExportStep::Open);
        s.settled();
        s.send(ExportStep::Check {
            stick: 0,
            playlists: vec![s.mix],
        });
        assert!(matches!(s.settled(), ExportView::Summary(_)));
    }

    fn job_of(s: &Setup) -> Option<ui::JobView> {
        s.parts.export.as_ref().unwrap().job(Instant::now())
    }

    fn copies(s: &Setup) -> usize {
        let music = s.stick.join("Open Jockey").join("Music");
        let mut n = 0;
        let mut todo = vec![music];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    todo.push(entry.path());
                } else if !entry.path().to_string_lossy().ends_with(".ojpart") {
                    n += 1;
                }
            }
        }
        n
    }

    fn job_op(s: &mut Setup, op: ui::JobOp) {
        s.window
            .send(UiCommand::Job {
                kind: ui::JobKind::Export,
                op,
            })
            .unwrap();
        commands::drain(&s.commands, &mut s.parts, None);
    }

    #[test]
    fn a_paused_export_copies_nothing_until_resumed() {
        let mut s = setup();
        with_tracks_ready(&mut s, &["b.wav", "c.wav", "d.wav"]);
        s.send(ExportStep::Write);
        let pill = job_of(&s).expect("the pill shows while writing");
        assert_eq!(pill.kind, ui::JobKind::Export);
        assert_eq!(pill.label, "Exporting to STICK");
        assert_eq!(pill.total, Some(4));
        job_op(&mut s, ui::JobOp::Pause);
        // The file in flight finishes, then the export waits: "pausing", then "paused".
        let began = Instant::now();
        while job_of(&s).unwrap().state != ui::JobState::Paused {
            assert!(began.elapsed() < Duration::from_secs(10), "never paused");
            std::thread::sleep(Duration::from_millis(5));
        }
        let held = copies(&s);
        assert!(held < 4, "{held} copies");
        std::thread::sleep(Duration::from_millis(200));
        commands::drain(&s.commands, &mut s.parts, None);
        assert_eq!(copies(&s), held, "nothing is copied while paused");
        assert!(matches!(s.view(), ExportView::Writing { .. }));
        assert_eq!(job_of(&s).unwrap().eta, None);

        job_op(&mut s, ui::JobOp::Resume);
        assert!(matches!(s.settled(), ExportView::Done(_)));
        assert_eq!(copies(&s), 4);
        assert!(job_of(&s).is_none());
    }

    #[test]
    fn a_stop_while_paused_ends_the_export_with_an_event() {
        let mut s = setup();
        with_tracks_ready(&mut s, &["b.wav", "c.wav", "d.wav"]);
        s.send(ExportStep::Write);
        job_op(&mut s, ui::JobOp::Pause);
        let began = Instant::now();
        while job_of(&s).unwrap().state != ui::JobState::Paused {
            assert!(began.elapsed() < Duration::from_secs(10), "never paused");
            std::thread::sleep(Duration::from_millis(5));
        }
        let held = copies(&s);
        job_op(&mut s, ui::JobOp::Stop);
        let ExportView::Done(text) = s.settled() else {
            panic!("not done: {:?}", s.view());
        };
        let expected = format!(
            "Export stopped: {held} of 4 files copied; the stick keeps its previous playlists"
        );
        assert_eq!(text, expected);
        assert_eq!(
            s.parts.events.iter().map(|(_, t)| t.as_str()).next_back(),
            Some(expected.as_str())
        );
        assert_eq!(copies(&s), held);
        // No library file was written: the stick has no crates yet.
        assert!(!s.stick.join("_Serato_").join("Subcrates").exists());
    }

    #[test]
    fn check_then_write_reaches_done() {
        let mut s = setup();
        assert_eq!(s.send(ExportStep::Open), ExportView::Listing);
        let ExportView::Choose(choice) = s.settled() else {
            panic!("no choice: {:?}", s.view());
        };
        let names: Vec<&str> = choice.sticks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["STICK"]);
        assert!(choice.sticks[0].exported.is_empty());
        assert_eq!(
            choice.playlists,
            [ui::PlaylistLabel {
                id: s.mix,
                name: "Mix".to_owned(),
                tracks: 1,
            }]
        );
        s.send(ExportStep::Check {
            stick: 0,
            playlists: vec![s.mix],
        });
        let ExportView::Summary(summary) = s.settled() else {
            panic!("no summary: {:?}", s.view());
        };
        assert_eq!(
            (summary.stick.as_str(), summary.playlists, summary.tracks),
            ("STICK", 1, 1)
        );
        assert_eq!((summary.to_copy, summary.kept), (1, 0));
        assert!(!summary.too_small, "{summary:?}");
        assert!(!s.manifest().exists(), "the check writes nothing");
        s.send(ExportStep::Write);
        let ExportView::Done(text) = s.settled() else {
            panic!("not done: {:?}", s.view());
        };
        assert!(
            text.starts_with(
                "Exported 1 playlist to STICK: 1 copied, 0 kept.\n\
                 To open them in Rekordbox (also in IMPORT TO REKORDBOX.txt on the stick):\n1. In Rekordbox,"
            ),
            "{text}"
        );
        assert!(text.contains("\n3. In the tree on the left"), "{text}");
        assert!(s.manifest().is_file());
        assert_eq!(s.send(ExportStep::Close), ExportView::Idle);
        // Listed again, the stick holds the playlist: it comes ticked.
        s.send(ExportStep::Open);
        let ExportView::Choose(choice) = s.settled() else {
            panic!("no choice: {:?}", s.view());
        };
        assert_eq!(choice.sticks[0].exported, [s.mix]);
    }

    #[test]
    fn write_without_check_is_ignored() {
        let mut s = setup();
        assert_eq!(s.send(ExportStep::Write), ExportView::Idle);
        s.send(ExportStep::Open);
        let choose = s.settled();
        assert!(matches!(choose, ExportView::Choose(_)), "{choose:?}");
        assert_eq!(s.send(ExportStep::Write), choose);
        assert_eq!(s.settled(), choose);
        assert!(!s.manifest().exists());
        assert_eq!(std::fs::read_dir(&s.stick).unwrap().count(), 0);
    }

    #[test]
    fn back_returns_to_the_choice_and_close_puts_it_away() {
        let mut s = setup();
        s.send(ExportStep::Open);
        let choose = s.settled();
        s.send(ExportStep::Check {
            stick: 0,
            playlists: vec![s.mix],
        });
        assert!(matches!(s.settled(), ExportView::Summary(_)));
        assert_eq!(s.send(ExportStep::Back), choose);
        assert_eq!(s.send(ExportStep::Close), ExportView::Idle);
        assert_eq!(std::fs::read_dir(&s.stick).unwrap().count(), 0);
    }

    #[test]
    fn a_check_of_a_stick_not_listed_is_ignored() {
        let mut s = setup();
        s.send(ExportStep::Open);
        let choose = s.settled();
        assert_eq!(
            s.send(ExportStep::Check {
                stick: 5,
                playlists: vec![s.mix],
            }),
            choose
        );
    }

    #[test]
    fn a_damaged_manifest_fails_the_check() {
        let mut s = setup();
        std::fs::create_dir_all(s.manifest().parent().unwrap()).unwrap();
        std::fs::write(s.manifest(), "not = [valid").unwrap();
        s.send(ExportStep::Open);
        let ExportView::Choose(choice) = s.settled() else {
            panic!("no choice: {:?}", s.view());
        };
        assert!(choice.sticks[0].exported.is_empty());
        s.send(ExportStep::Check {
            stick: 0,
            playlists: vec![s.mix],
        });
        let failed = s.settled();
        assert!(
            matches!(&failed, ExportView::Failed(why)
                if why.starts_with("Open Jockey's manifest on the stick is damaged")),
            "{failed:?}"
        );
        assert_eq!(s.send(ExportStep::Close), ExportView::Idle);
    }
}
