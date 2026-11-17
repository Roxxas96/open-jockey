//! The guest sticks plugged in right now: USB sticks prepared by another DJ app, read-only.
//!
//! Each track of a stick gets a session id below zero, so it can never be mistaken for one
//! of the owner's library tracks (the library refuses negative ids). Nothing here touches the
//! owner's library or writes to a stick.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use engine::{HOTCUES, TrackMemory};
use interop::guest::{GuestApp, GuestStick, GuestTrack};
use library::{GuestBranch, GuestList, GuestNode, TrackRow};
use ui::HotcueMark;

use crate::format::safe;

/// What an app's branch is called in the browser.
fn app_label(app: GuestApp) -> &'static str {
    match app {
        GuestApp::Serato => "Serato",
        GuestApp::RekordboxDevice => "rekordbox",
        GuestApp::RekordboxXml => "rekordbox (xml)",
        GuestApp::VirtualDj => "VirtualDJ",
    }
}

/// A track's hotcue colours and names, slot by slot.
pub type HotcueMarks = [Option<HotcueMark>; HOTCUES];

/// A stick as the browser lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct StickSummary {
    pub root: PathBuf,
    pub name: String,
    /// One per app on the stick, in the order the apps were read.
    pub sources: Vec<SourceSummary>,
}

/// One app's branch of a stick.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSummary {
    pub app: GuestApp,
    /// Why the app's data could not be read in full.
    pub problem: Option<String>,
    /// The ids of the app's tracks, in the app's order.
    pub tracks: Vec<i64>,
    /// Each playlist's name and the ids of its tracks, in order.
    pub playlists: Vec<(String, Vec<i64>)>,
}

/// Everything a loader needs of a guest track.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestTrackRef {
    /// The browser row, with the track's negative id.
    pub row: TrackRow,
    /// On the stick, absolute.
    pub path: PathBuf,
    pub track: GuestTrack,
    pub app: GuestApp,
    /// The stick's mount path and name.
    pub root: PathBuf,
    pub stick_name: String,
}

/// The sticks plugged in, shared (behind an `Arc`) between the control thread and the
/// library-worker. The lock is held only to copy.
#[derive(Debug, Default)]
pub struct GuestSticks {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Ids handed out so far: the next is `-(issued + 1)`.
    issued: i64,
    /// Sticks being read, with the number of the plug-in that asked.
    expected: HashMap<PathBuf, u64>,
    plugs: u64,
    /// Grows with every stick added or removed.
    generation: u64,
    sticks: Vec<StickSummary>,
    tracks: HashMap<i64, GuestTrackRef>,
    hotcues: HashMap<i64, TrackMemory>,
    /// The session's hotcue colours and names, per track.
    marks: HashMap<i64, HotcueMarks>,
    /// The sticks removed, as the lowest and highest id they had and their name.
    gone: Vec<(i64, i64, String)>,
}

impl GuestSticks {
    /// Whether `id` is a guest track's: a library id is never below zero.
    pub fn is_guest(id: i64) -> bool {
        id < 0
    }

    /// The sticks, in the order they were read.
    pub fn sticks(&self) -> Vec<StickSummary> {
        self.lock().sticks.clone()
    }

    /// The sticks as the browser lists them: each app's branch with its lists, the names
    /// cleaned of control characters. Reads the rows one list at a time, so no lock is held
    /// long.
    pub fn nodes(&self) -> Vec<GuestNode> {
        let sticks = self.sticks();
        sticks
            .into_iter()
            .map(|stick| GuestNode {
                name: safe(&stick.name),
                root: stick.root,
                branches: stick
                    .sources
                    .iter()
                    .map(|source| GuestBranch {
                        label: app_label(source.app).to_owned(),
                        note: source.problem.as_ref().map(safe),
                        lists: source
                            .playlists
                            .iter()
                            .map(|(name, ids)| GuestList {
                                name: safe(name),
                                rows: self.rows(ids),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// How many times the set of sticks changed: a stick was added or removed.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// The path and browser row of each of `ids` still plugged in, in order, under one
    /// short lock.
    pub fn rows(&self, ids: &[i64]) -> Vec<(PathBuf, TrackRow)> {
        let inner = self.lock();
        ids.iter()
            .filter_map(|id| inner.tracks.get(id))
            .map(|t| (t.path.clone(), t.row.clone()))
            .collect()
    }

    /// The guest track `id`, or `None` for an unknown id or one whose stick was removed.
    pub fn track(&self, id: i64) -> Option<GuestTrackRef> {
        self.lock().tracks.get(&id).cloned()
    }

    /// Keeps `memory` as the session's hotcues of guest track `id`; it dies with the stick.
    /// An id that is not a track of a plugged-in stick is ignored.
    pub fn set_hotcues(&self, id: i64, memory: TrackMemory) {
        let mut inner = self.lock();
        if inner.tracks.contains_key(&id) {
            inner.hotcues.insert(id, memory);
        }
    }

    /// The session's hotcues of guest track `id` ([`set_hotcues`](Self::set_hotcues)), which
    /// win over the stick's own; `None` when there are none or the stick was removed.
    pub fn hotcues(&self, id: i64) -> Option<TrackMemory> {
        self.lock().hotcues.get(&id).copied()
    }

    /// Keeps `marks` as the session's hotcue colours and names of guest track `id`, as
    /// [`set_hotcues`](Self::set_hotcues) keeps its hotcues.
    pub fn set_marks(&self, id: i64, marks: HotcueMarks) {
        let mut inner = self.lock();
        if inner.tracks.contains_key(&id) {
            inner.marks.insert(id, marks);
        }
    }

    /// The session's hotcue colours and names of guest track `id`
    /// ([`set_marks`](Self::set_marks)).
    pub fn marks(&self, id: i64) -> Option<HotcueMarks> {
        self.lock().marks.get(&id).cloned()
    }

    /// The name of the removed stick guest track `id` was on; `None` when it was never
    /// handed out or its stick is still plugged in.
    pub fn gone_stick(&self, id: i64) -> Option<String> {
        let inner = self.lock();
        inner
            .gone
            .iter()
            .find(|(low, high, _)| (*low..=*high).contains(&id))
            .map(|(_, _, name)| name.clone())
    }

    /// Notes that the stick at `root` is being read; the number goes back with the result.
    pub fn expect(&self, root: &Path) -> u64 {
        let mut inner = self.lock();
        inner.plugs += 1;
        let plug = inner.plugs;
        inner.expected.insert(root.to_path_buf(), plug);
        plug
    }

    /// Adds a stick read for plug-in `plug` ([`expect`](Self::expect)), unless it was removed
    /// meanwhile. Says whether it was added.
    pub fn add(&self, stick: &GuestStick, plug: u64) -> bool {
        let mut inner = self.lock();
        if inner.expected.get(&stick.root) != Some(&plug) {
            return false;
        }
        inner.expected.remove(&stick.root);
        let mut sources = Vec::new();
        for source in &stick.sources {
            let ids: Vec<i64> = source
                .tracks
                .iter()
                .map(|track| {
                    inner.issued += 1;
                    let id = -inner.issued;
                    let row = TrackRow {
                        id,
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        bpm: track.bpm,
                        key: None,
                        duration_frames: None,
                        sample_rate: None,
                        has_cues: track.main_cue.is_some() || !track.hotcues.is_empty(),
                        missing: false,
                    };
                    inner.tracks.insert(
                        id,
                        GuestTrackRef {
                            row,
                            path: track.path.clone(),
                            track: track.clone(),
                            app: source.app,
                            root: stick.root.clone(),
                            stick_name: stick.name.clone(),
                        },
                    );
                    id
                })
                .collect();
            let playlists = source
                .playlists
                .iter()
                .map(|p| {
                    let tracks = p.tracks.iter().filter_map(|&i| ids.get(i).copied());
                    (p.name.clone(), tracks.collect())
                })
                .collect();
            sources.push(SourceSummary {
                app: source.app,
                problem: source.problem.clone(),
                tracks: ids,
                playlists,
            });
        }
        inner.sticks.push(StickSummary {
            root: stick.root.clone(),
            name: stick.name.clone(),
            sources,
        });
        inner.generation += 1;
        true
    }

    /// Forgets the stick at `root`, its ids and its session edits. Returns its name when it
    /// had been added.
    pub fn remove(&self, root: &Path) -> Option<String> {
        let mut inner = self.lock();
        inner.expected.remove(root);
        let at = inner.sticks.iter().position(|s| s.root == root)?;
        let stick = inner.sticks.remove(at);
        inner.generation += 1;
        let ids = stick.sources.iter().flat_map(|s| &s.tracks);
        if let (Some(&low), Some(&high)) = (ids.clone().min(), ids.clone().max()) {
            inner.gone.push((low, high, stick.name.clone()));
        }
        for id in ids {
            inner.tracks.remove(id);
            inner.hotcues.remove(id);
            inner.marks.remove(id);
        }
        Some(stick.name)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interop::guest::{GuestPlaylist, GuestSource};

    fn track(path: &str) -> GuestTrack {
        GuestTrack {
            path: PathBuf::from(path),
            title: Some("T".into()),
            artist: None,
            album: None,
            bpm: Some(120.0),
            first_beat: None,
            main_cue: None,
            hotcues: Vec::new(),
        }
    }

    fn source(app: GuestApp, paths: &[&str]) -> GuestSource {
        GuestSource {
            app,
            tracks: paths.iter().map(|p| track(p)).collect(),
            playlists: vec![GuestPlaylist {
                name: "P".into(),
                tracks: (0..paths.len()).collect(),
            }],
            problem: None,
        }
    }

    fn stick(root: &str, sources: Vec<GuestSource>) -> GuestStick {
        GuestStick {
            root: PathBuf::from(root),
            name: "S".into(),
            sources,
        }
    }

    fn plug(store: &GuestSticks, stick: &GuestStick) {
        let plug = store.expect(&stick.root);
        assert!(store.add(stick, plug));
    }

    #[test]
    fn the_same_file_in_two_apps_has_two_ids() {
        let store = GuestSticks::default();
        let file = "/mnt/S/song.wav";
        plug(
            &store,
            &stick(
                "/mnt/S",
                vec![
                    source(GuestApp::Serato, &[file]),
                    source(GuestApp::VirtualDj, &[file]),
                ],
            ),
        );
        let sticks = store.sticks();
        let (a, b) = (
            sticks[0].sources[0].tracks[0],
            sticks[0].sources[1].tracks[0],
        );
        assert_ne!(a, b);
        assert_eq!(store.track(a).unwrap().path, store.track(b).unwrap().path);
        assert_eq!(store.track(a).unwrap().app, GuestApp::Serato);
        assert_eq!(store.track(b).unwrap().app, GuestApp::VirtualDj);
        assert_eq!(store.track(a).unwrap().row.id, a);
        assert_eq!(store.track(a).unwrap().root, PathBuf::from("/mnt/S"));
    }

    #[test]
    fn guest_ids_are_negative_and_never_reused() {
        let store = GuestSticks::default();
        let s = stick(
            "/mnt/S",
            vec![source(GuestApp::Serato, &["/mnt/S/a", "/mnt/S/b"])],
        );
        plug(&store, &s);
        let first = store.sticks()[0].sources[0].tracks.clone();
        assert!(first.iter().all(|&id| id < 0 && GuestSticks::is_guest(id)));
        assert!(!GuestSticks::is_guest(0) && !GuestSticks::is_guest(7));
        store.set_hotcues(first[0], TrackMemory::default());
        assert_eq!(store.remove(&s.root).as_deref(), Some("S"));
        assert!(store.track(first[0]).is_none());
        // A removed stick's ids still name it; one never handed out names none.
        assert_eq!(store.gone_stick(first[1]).as_deref(), Some("S"));
        assert_eq!(store.gone_stick(-99), None);
        assert!(store.sticks().is_empty());
        plug(&store, &s);
        let second = store.sticks()[0].sources[0].tracks.clone();
        assert!(second.iter().all(|id| !first.contains(id) && *id < 0));
        // A session edit died with the stick.
        assert!(store.lock().hotcues.is_empty());
    }

    #[test]
    fn a_stick_removed_while_it_was_read_is_not_added() {
        let store = GuestSticks::default();
        let s = stick("/mnt/S", vec![source(GuestApp::Serato, &["/mnt/S/a"])]);
        let plug = store.expect(&s.root);
        assert_eq!(store.remove(&s.root), None);
        assert!(!store.add(&s, plug));
        assert!(store.sticks().is_empty());
    }

    #[test]
    fn hotcues_are_kept_for_the_session() {
        let store = GuestSticks::default();
        let s = stick("/mnt/S", vec![source(GuestApp::Serato, &["/mnt/S/a"])]);
        plug(&store, &s);
        let id = store.sticks()[0].sources[0].tracks[0];
        let memory = TrackMemory {
            cue: 5.0,
            ..TrackMemory::default()
        };
        store.set_hotcues(id, memory);
        assert_eq!(store.lock().hotcues.get(&id), Some(&memory));
        // An unknown id is ignored.
        store.set_hotcues(-99, memory);
        assert_eq!(store.lock().hotcues.len(), 1);
    }
}
