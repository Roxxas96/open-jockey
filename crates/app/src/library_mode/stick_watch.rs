//! Finding guest sticks as they are plugged in and removed. The library-worker checks the
//! mounts every [`CHECK_EVERY`] (a hung mount can delay that check on the worker, which
//! lists only names); a new stick is read on a "guest-reader" thread, so reading a slow
//! stick never stalls browsing or loading, and lands in the shared [`GuestSticks`].

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use interop::guest::{GuestApp, GuestStick};
use library::{Library, Reader, guest_markers};

use super::worker::Event;
use crate::format::safe;
use crate::guest::GuestSticks;
use crate::sticks::{Mounts, library_volumes, mounted_volumes};

/// How often the mounts are looked at.
pub const CHECK_EVERY: Duration = Duration::from_secs(2);

/// A stick to read: its mount path, its name and the plug-in number of the store.
type ReadJob = (PathBuf, String, u64);

/// How a stick is read (the real reader, or a stand-in in tests).
type ReadFn = fn(&Path, &str) -> GuestStick;

/// A stick's name in a message: the volume's name, or "drive E" for a drive letter.
fn display_name(name: &str) -> String {
    let mut chars = name.chars();
    match (chars.next(), chars.next(), chars.as_str()) {
        (Some(letter), Some(':'), "" | "\\" | "/") if letter.is_ascii_alphabetic() => {
            format!("drive {}", letter.to_ascii_uppercase())
        }
        _ => name.to_owned(),
    }
}

/// Looks at the mounts for sticks that came or went. Lives on the library-worker thread.
pub struct StickWatcher {
    store: Arc<GuestSticks>,
    mounts: Mounts,
    events: Sender<Event>,
    /// Reads the library folders, to tell the owner's drive from a guest stick.
    folders: Option<Reader>,
    /// The sticks handed to the store (or being read): mount path and name.
    known: HashMap<PathBuf, String>,
    /// To the guest-reader thread; `None` when it could not be started.
    reads: Option<Sender<ReadJob>>,
}

impl StickWatcher {
    /// Starts the guest-reader thread; it ends when the watcher is dropped.
    pub fn start(
        lib: &Library,
        events: &Sender<Event>,
        mounts: Mounts,
        store: Arc<GuestSticks>,
    ) -> Self {
        Self::start_with(lib, events, mounts, store, interop::guest::read_stick)
    }

    /// [`start`](Self::start) reading sticks with `read` (a stand-in in tests).
    fn start_with(
        lib: &Library,
        events: &Sender<Event>,
        mounts: Mounts,
        store: Arc<GuestSticks>,
        read: ReadFn,
    ) -> Self {
        let (reads, jobs) = channel();
        let (reader_store, reader_events) = (Arc::clone(&store), events.clone());
        let thread = std::thread::Builder::new()
            .name("guest-reader".into())
            .spawn(move || read_sticks(&jobs, &reader_store, &reader_events, read));
        if let Err(e) = &thread {
            let _ = events.send(Event::Message(format!(
                "error: cannot start the guest stick reader: {e}"
            )));
        }
        Self {
            store,
            mounts,
            events: events.clone(),
            folders: lib.reader().ok(),
            known: HashMap::new(),
            reads: thread.ok().map(|_| reads),
        }
    }

    /// Lists the mounts once: reports sticks that left, and hands new ones to the reader.
    /// A volume counts when it is not the drive of a library folder and carries an app
    /// marker. A stick that left is dropped even when the library folders cannot be read;
    /// when they cannot, nothing new is taken.
    pub fn scan(&mut self) {
        let volumes = mounted_volumes(&self.mounts);
        let owner: Option<HashSet<PathBuf>> = self
            .folders
            .as_ref()
            .and_then(|r| r.folders().ok())
            .map(|folders| {
                library_volumes(&folders, &self.mounts)
                    .into_iter()
                    .map(|(volume, _)| volume)
                    .collect()
            });
        // The owner's drive first: it costs no stats.
        let is_guest = |path: &Path| {
            !owner.as_ref().is_some_and(|o| o.contains(path)) && guest_markers(path).any()
        };
        let gone: Vec<PathBuf> = self
            .known
            .keys()
            .filter(|root| {
                let still = volumes.iter().any(|(path, _)| path == *root);
                !(still && is_guest(root))
            })
            .cloned()
            .collect();
        for root in gone {
            self.known.remove(&root);
            if let Some(name) = self.store.remove(&root) {
                let _ = self.events.send(Event::Message(format!(
                    "Stick {} removed",
                    safe(display_name(&name))
                )));
            }
        }
        let (Some(_), Some(reads)) = (&owner, &self.reads) else {
            return;
        };
        for (root, name) in volumes {
            if self.known.contains_key(&root) || !is_guest(&root) {
                continue;
            }
            let name = display_name(&name);
            let plug = self.store.expect(&root);
            if reads.send((root.clone(), name.clone(), plug)).is_ok() {
                self.known.insert(root, name);
            }
        }
    }
}

/// What the stick's apps are called in a message.
fn app_name(app: GuestApp) -> &'static str {
    match app {
        GuestApp::Serato => "Serato",
        GuestApp::RekordboxDevice | GuestApp::RekordboxXml => "rekordbox",
        GuestApp::VirtualDj => "VirtualDJ",
    }
}

/// The guest-reader thread: reads each stick (no lock held) and adds it unless it was
/// removed meanwhile, then says what it found and what could not be read.
fn read_sticks(
    jobs: &Receiver<ReadJob>,
    store: &GuestSticks,
    events: &Sender<Event>,
    read: ReadFn,
) {
    for (root, name, plug) in jobs {
        let say = |text: String| {
            let _ = events.send(Event::Message(text));
        };
        // A crafted stick that crashes the reader must not end it: the sticks after it
        // are still found.
        let Ok(stick) = catch_unwind(AssertUnwindSafe(|| read(&root, &name))) else {
            say(format!("Stick {}: could not be read", safe(&name)));
            continue;
        };
        if !store.add(&stick, plug) {
            continue;
        }
        let mut apps: Vec<&str> = Vec::new();
        for source in &stick.sources {
            let app = app_name(source.app);
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        if !apps.is_empty() {
            say(format!("Stick {}: {}", safe(&name), apps.join(", ")));
        }
        for problem in stick.sources.iter().filter_map(|s| s.problem.as_ref()) {
            say(format!("Stick {}: {}", safe(&name), safe(problem)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    struct Rig {
        _dir: tempfile::TempDir,
        /// The mount dirs.
        mnt: Vec<PathBuf>,
        lib: Library,
        store: Arc<GuestSticks>,
        events: Receiver<Event>,
        watcher: StickWatcher,
    }

    /// `mounts` mount dirs, a library, and a watcher over them.
    fn rig(mounts: usize) -> Rig {
        rig_reading(mounts, interop::guest::read_stick)
    }

    /// [`rig`] with the stick reader `read`.
    fn rig_reading(mounts: usize, read: ReadFn) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(dir.path()).unwrap();
        let mnt: Vec<PathBuf> = (0..mounts)
            .map(|i| {
                let m = base.join(format!("mnt{i}"));
                std::fs::create_dir_all(&m).unwrap();
                m
            })
            .collect();
        let lib = Library::open(&base.join("library.sqlite3")).unwrap();
        let store = Arc::new(GuestSticks::default());
        let (tx, events) = channel();
        let mounts = Mounts {
            dirs: mnt.clone(),
            drive_letters: false,
        };
        let watcher = StickWatcher::start_with(&lib, &tx, mounts, Arc::clone(&store), read);
        Rig {
            _dir: dir,
            mnt,
            lib,
            store,
            events,
            watcher,
        }
    }

    /// Waits up to 20 s for the message `text`, skipping others.
    fn wait_message(events: &Receiver<Event>, text: &str) {
        loop {
            match events.recv_timeout(Duration::from_secs(20)) {
                Ok(Event::Message(m)) if m == text => return,
                Ok(_) => {}
                Err(_) => panic!("no message {text:?} within 20 s"),
            }
        }
    }

    fn guest_stick(mnt: &Path) -> PathBuf {
        let stick = mnt.join("OJGUEST");
        copy_dir(Path::new(GUEST), &stick);
        stick
    }

    const FOUND: &str = "Stick OJGUEST: Serato, rekordbox, VirtualDJ";

    #[test]
    fn a_marked_volume_becomes_a_guest_stick() {
        let mut rig = rig(1);
        let stick = guest_stick(&rig.mnt[0]);
        std::fs::create_dir(rig.mnt[0].join("PLAIN")).unwrap();
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        let sticks = rig.store.sticks();
        assert_eq!(sticks.len(), 1);
        assert_eq!(sticks[0].root, stick);
        assert_eq!(sticks[0].name, "OJGUEST");
        assert!(sticks[0].sources.iter().all(|s| !s.tracks.is_empty()));
        // Another look finds nothing new.
        rig.watcher.scan();
        assert_eq!(rig.store.sticks().len(), 1);
    }

    #[test]
    fn the_owners_music_drive_is_never_a_guest() {
        let mut rig = rig(1);
        let music = rig.mnt[0].join("MUSIC");
        std::fs::create_dir_all(music.join("VirtualDJ")).unwrap();
        std::fs::create_dir_all(music.join("songs")).unwrap();
        library::add_folder(&rig.lib, &music.join("songs")).unwrap();
        guest_stick(&rig.mnt[0]);
        rig.watcher.scan();
        // Both were looked at in one scan; the real stick shows, the music drive does not.
        assert!(!rig.watcher.known.contains_key(&music));
        assert!(rig.watcher.known.contains_key(&rig.mnt[0].join("OJGUEST")));
        wait_message(&rig.events, FOUND);
        let sticks = rig.store.sticks();
        assert_eq!(sticks.len(), 1);
        assert_eq!(sticks[0].name, "OJGUEST");
    }

    #[test]
    fn removing_the_stick_removes_it() {
        let mut rig = rig(1);
        let stick = guest_stick(&rig.mnt[0]);
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        let id = rig.store.sticks()[0].sources[0].tracks[0];
        std::fs::remove_dir_all(&stick).unwrap();
        rig.watcher.scan();
        wait_message(&rig.events, "Stick OJGUEST removed");
        assert!(rig.store.sticks().is_empty());
        assert!(rig.store.track(id).is_none());
        // Plugged in again: found again, with new ids.
        guest_stick(&rig.mnt[0]);
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        assert!(
            rig.store.sticks()[0].sources[0]
                .tracks
                .iter()
                .all(|&t| t < id)
        );
    }

    #[test]
    fn a_removed_stick_goes_even_when_the_library_folders_cannot_be_read() {
        let mut rig = rig(1);
        let stick = guest_stick(&rig.mnt[0]);
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        rig.watcher.folders = None;
        std::fs::remove_dir_all(&stick).unwrap();
        rig.watcher.scan();
        wait_message(&rig.events, "Stick OJGUEST removed");
        assert!(rig.store.sticks().is_empty());
    }

    /// Reads like the real reader, except a stick called BAD, which panics.
    fn flaky(root: &Path, name: &str) -> interop::guest::GuestStick {
        assert_ne!(name, "BAD", "a crafted stick");
        interop::guest::read_stick(root, name)
    }

    #[test]
    fn a_stick_that_panics_the_reader_does_not_stop_detection() {
        let mut rig = rig_reading(1, flaky);
        let bad = rig.mnt[0].join("BAD");
        std::fs::create_dir_all(bad.join("VirtualDJ")).unwrap();
        rig.watcher.scan();
        wait_message(&rig.events, "Stick BAD: could not be read");
        assert!(rig.store.sticks().is_empty());
        guest_stick(&rig.mnt[0]);
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        assert_eq!(rig.store.sticks().len(), 1);
    }

    #[test]
    fn a_drive_letter_stick_is_named_drive_e() {
        assert_eq!(display_name("E:\\"), "drive E");
        assert_eq!(display_name("e:"), "drive E");
        assert_eq!(display_name("OJGUEST"), "OJGUEST");
        assert_eq!(display_name("E:x"), "E:x");
    }

    #[test]
    fn two_sticks_with_one_name_stay_apart() {
        let mut rig = rig(2);
        let (a, b) = (guest_stick(&rig.mnt[0]), guest_stick(&rig.mnt[1]));
        rig.watcher.scan();
        wait_message(&rig.events, FOUND);
        wait_message(&rig.events, FOUND);
        let sticks = rig.store.sticks();
        assert_eq!(sticks.len(), 2);
        assert_ne!(sticks[0].root, sticks[1].root);
        assert_ne!(sticks[0].sources[0].tracks, sticks[1].sources[0].tracks);
        std::fs::remove_dir_all(&a).unwrap();
        rig.watcher.scan();
        wait_message(&rig.events, "Stick OJGUEST removed");
        let left = rig.store.sticks();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].root, b);
    }

    #[test]
    fn a_source_problem_is_reported() {
        let mut rig = rig(1);
        let stick = rig.mnt[0].join("BAD");
        std::fs::create_dir_all(stick.join("PIONEER").join("rekordbox")).unwrap();
        std::fs::write(
            stick.join("PIONEER").join("rekordbox").join("export.pdb"),
            b"junk",
        )
        .unwrap();
        rig.watcher.scan();
        let mut seen = Vec::new();
        while seen.len() < 2 {
            match rig.events.recv_timeout(Duration::from_secs(20)) {
                Ok(Event::Message(m)) if m.starts_with("Stick BAD: ") => seen.push(m),
                Ok(_) => {}
                Err(_) => panic!("no problem message: {seen:?}"),
            }
        }
        assert!(seen.iter().any(|m| m == "Stick BAD: rekordbox"), "{seen:?}");
        assert!(
            seen.iter()
                .any(|m| m.starts_with("Stick BAD: ") && m != "Stick BAD: rekordbox"),
            "{seen:?}"
        );
    }
}
