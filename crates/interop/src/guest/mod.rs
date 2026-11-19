//! Reads a USB stick prepared by another DJ app (a "guest stick"), read-only.
//!
//! Nothing here writes to the stick, and nothing read is added to the owner's library.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::export::FileKind;
use crate::export::timing::{App, ours_seconds};

pub(crate) mod anlz;
mod pdb;
mod rekordbox_device;
mod rekordbox_xml;
mod serato;
mod virtualdj;

/// The app that prepared a stick's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestApp {
    Serato,
    /// rekordbox's device library (`PIONEER/rekordbox/export.pdb`).
    RekordboxDevice,
    /// rekordbox's `rekordbox.xml` export.
    RekordboxXml,
    VirtualDj,
}

/// A mounted stick and what each app on it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestStick {
    pub root: PathBuf,
    pub name: String,
    pub sources: Vec<GuestSource>,
}

/// One app's view of a stick.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestSource {
    pub app: GuestApp,
    pub tracks: Vec<GuestTrack>,
    pub playlists: Vec<GuestPlaylist>,
    /// Why this app's data could not be read in full; shown on its branch.
    pub problem: Option<String>,
}

/// A playlist (a crate for Serato); nested folders are flattened to `Folder / Playlist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestPlaylist {
    pub name: String,
    /// Indexes into the source's `tracks`, in order.
    pub tracks: Vec<usize>,
}

/// A track on the stick with what its app knows about it.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestTrack {
    /// On the stick, absolute.
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub bpm: Option<f64>,
    /// Seconds.
    pub first_beat: Option<f64>,
    /// Seconds.
    pub main_cue: Option<f64>,
    pub hotcues: Vec<GuestCue>,
}

/// A hotcue, or a saved loop when `loop_seconds` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestCue {
    /// 0..8.
    pub slot: u8,
    pub seconds: f64,
    pub loop_seconds: Option<f64>,
    /// `0xRRGGBB`.
    pub color: Option<u32>,
    pub name: Option<String>,
}

/// An app's reader: the source for a stick, or `None` when the app's marker is absent.
type SourceReader = fn(&Path, &library::GuestMarkers) -> Option<GuestSource>;

/// Every app's reader. A new app is one more entry; each checks its own marker.
const READERS: &[SourceReader] = &[
    serato::read,
    rekordbox_device::read,
    rekordbox_xml::read,
    virtualdj::read,
];

/// Reads every app whose marker is on the stick. A failing app sets its source's `problem`
/// and never stops the others.
#[must_use]
pub fn read_stick(root: &Path, name: &str) -> GuestStick {
    let markers = library::guest_markers(root);
    let sources = READERS
        .iter()
        .filter_map(|read| read(root, &markers))
        .collect();
    GuestStick {
        root: root.to_path_buf(),
        name: name.to_owned(),
        sources,
    }
}

/// The largest database file read whole (Serato's `database V2`, VirtualDJ's `database.xml`).
pub(crate) const MAX_DATABASE_BYTES: u64 = 256 * 1024 * 1024;
/// The largest playlist or order file read whole (a crate, `neworder.pref`, a `.vdjfolder`,
/// an `.m3u`, VirtualDJ's `order`).
pub(crate) const MAX_LIST_BYTES: u64 = 16 * 1024 * 1024;
/// The most parts a stored track path may have; a deeper one is refused unread, so mapping it
/// onto the stick stays cheap. FAT and exFAT paths are never this deep.
pub(crate) const MAX_PATH_PARTS: usize = 64;

/// Why a capped read gave no bytes.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// Larger than the cap; nothing was read.
    TooLarge,
    Io(std::io::Error),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("is too large to read"),
            Self::Io(e) => write!(f, "could not be read: {e}"),
        }
    }
}

/// The bytes of `path`, or [`ReadError::TooLarge`] when it holds more than `max`. The size is
/// checked before reading, and the read itself stops past `max`, so a file that grows or a
/// damaged entry that misreports its size cannot make it unbounded.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, ReadError> {
    let file = std::fs::File::open(path).map_err(ReadError::Io)?;
    if file.metadata().map_err(ReadError::Io)?.len() > max {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > max {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

/// The deepest playlist folder nesting followed.
pub(crate) const MAX_DEPTH: usize = 32;
/// The most playlists built from one source.
pub(crate) const MAX_PLAYLISTS: usize = 10_000;
/// The most bytes one source's flattened playlist names take together.
pub(crate) const MAX_NAME_BYTES: usize = 32 * 1024 * 1024;
/// The longest flattened playlist name, in characters.
pub(crate) const MAX_NAME_CHARS: usize = 1024;

/// `prefix / name` (just `name` under no folder), cut to [`MAX_NAME_CHARS`] without building
/// the uncut name first.
pub(crate) fn join_name(prefix: &str, name: &str) -> String {
    let sep = if prefix.is_empty() { "" } else { " / " };
    prefix
        .chars()
        .chain(sep.chars())
        .chain(name.chars())
        .take(MAX_NAME_CHARS)
        .collect()
}

/// A track with only what its file's tags say: the title falls back to the file stem.
fn track_from_tags(path: PathBuf) -> GuestTrack {
    let tags = library::tags::read(&path);
    GuestTrack {
        path,
        title: Some(tags.title),
        artist: tags.artist,
        album: tags.album,
        bpm: None,
        first_beat: None,
        main_cue: None,
        hotcues: Vec::new(),
    }
}

/// Moves a rekordbox track's marks from rekordbox's timeline onto Open Jockey's: the first
/// beat, the main cue and every hotcue start, less the app's offset for the file (an MP3's
/// `lead_in` is read once). A mark that lands before our 0 is clamped to 0, not dropped; loop
/// lengths are durations and stay as they are. A file whose `lead_in` or rate cannot be read
/// gets no shift.
fn rekordbox_to_ours(track: &mut GuestTrack) {
    let kind = FileKind::of(&track.path);
    if kind != FileKind::Mp3 {
        return;
    }
    let (Ok(lead_in), Some(rate)) = (
        decode::lead_in(&track.path),
        library::tags::read(&track.path).sample_rate,
    ) else {
        return;
    };
    let ours =
        |seconds: f64| ours_seconds(App::Rekordbox, seconds, kind.clone(), &lead_in, rate).max(0.0);
    track.first_beat = track.first_beat.map(ours);
    track.main_cue = track.main_cue.map(ours);
    for cue in &mut track.hotcues {
        cue.seconds = ours(cue.seconds);
    }
}

/// `path` when it is a file inside the stick at `root`, else `None`. A path with a `..`
/// component, an absolute path elsewhere and a symlink out of the stick are all refused, so a
/// crafted stick cannot make the app read the owner's own files as guest tracks.
///
/// The check is on the canonical path, but the path returned is `path` as given, not the
/// canonical one: it may pass through links inside the stick, so a caller must not treat it
/// as canonical (canonicalise it first when that matters).
pub(crate) fn on_stick(root: &Path, path: &Path) -> Option<PathBuf> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    let root = dunce::canonicalize(root).ok()?;
    let real = dunce::canonicalize(path).ok()?;
    (real.is_file() && real.starts_with(&root)).then(|| path.to_path_buf())
}

/// Whether `dir` is a directory inside the stick at `root` (a link out of it is not).
pub(crate) fn dir_on_stick(root: &Path, dir: &Path) -> bool {
    let (Ok(root), Ok(real)) = (dunce::canonicalize(root), dunce::canonicalize(dir)) else {
        return false;
    };
    real.is_dir() && real.starts_with(root)
}
