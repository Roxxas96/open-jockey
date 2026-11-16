//! Reads a USB stick prepared by another DJ app (a "guest stick"), read-only.
//!
//! Nothing here writes to the stick, and nothing read is added to the owner's library.

use std::path::{Path, PathBuf};

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
const READERS: &[SourceReader] = &[virtualdj::read];

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
