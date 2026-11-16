//! The VirtualDJ branch of a guest stick: `VirtualDJ/database.xml` and `VirtualDJ/MyLists`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::{GuestApp, GuestCue, GuestPlaylist, GuestSource, GuestTrack, track_from_tags};
use crate::virtualdj::{VdjPoi, VdjSong, read_database, read_playlists};

/// Slots of a guest hotcue; VirtualDJ's `Num` 1..=8 become slot 0..=7.
const SLOTS: u32 = 8;

/// Reads the stick's VirtualDJ data, or `None` without the `VirtualDJ` folder.
pub(super) fn read(root: &Path, markers: &library::GuestMarkers) -> Option<GuestSource> {
    if !markers.virtualdj {
        return None;
    }
    let dir = root.join("VirtualDJ");
    let database = dir.join("database.xml");
    let mut problems = Vec::new();
    let mut source = GuestSource {
        app: GuestApp::VirtualDj,
        tracks: Vec::new(),
        playlists: Vec::new(),
        problem: None,
    };
    let mut index: HashMap<PathBuf, usize> = HashMap::new();

    if database.is_file() {
        match read_database(&database, Some(root)) {
            Ok(songs) => {
                for song in songs.into_iter().filter(|s| s.path.is_file()) {
                    index.insert(song.path.clone(), source.tracks.len());
                    source.tracks.push(track(song));
                }
            }
            Err(e) => problems.push(e.to_string()),
        }
    }

    let lists = dir.join("MyLists");
    if lists.is_dir() {
        let order = lists.join("order");
        match read_playlists(&[lists], Some(order.as_path()).filter(|p| p.is_file())) {
            Ok((playlists, errors)) => {
                problems.extend(errors.iter().map(ToString::to_string));
                let mut dropped = 0;
                for list in playlists {
                    let mut tracks = Vec::new();
                    for raw in &list.paths {
                        let Some(path) = on_stick(raw, root) else {
                            dropped += 1;
                            continue;
                        };
                        let at = *index.entry(path.clone()).or_insert_with(|| {
                            source.tracks.push(track_from_tags(path));
                            source.tracks.len() - 1
                        });
                        tracks.push(at);
                    }
                    source.playlists.push(GuestPlaylist {
                        name: list.name,
                        tracks,
                    });
                }
                if dropped > 0 {
                    problems.push(format!(
                        "{dropped} playlist {} in VirtualDJ/MyLists point at files that are not on this stick",
                        if dropped == 1 { "entry" } else { "entries" }
                    ));
                }
            }
            Err(e) => problems.push(e.to_string()),
        }
    }
    if !problems.is_empty() {
        source.problem = Some(problems.join("; "));
    }
    Some(source)
}

/// A database song with its tags, grid and cues; tags the database lacks come from the file.
fn track(song: VdjSong) -> GuestTrack {
    let mut track = track_from_tags(song.path.clone());
    if song.title.is_some() {
        track.title = song.title;
    }
    if song.artist.is_some() {
        track.artist = song.artist;
    }
    if song.album.is_some() {
        track.album = song.album;
    }
    track.bpm = song.seconds_per_beat.map(|spb| 60.0 / spb);
    for poi in song.pois {
        let (num, seconds, loop_seconds, name, color) = match poi {
            VdjPoi::Hotcue {
                num,
                seconds,
                name,
                color,
            } => (num, seconds, None, name, color),
            VdjPoi::Loop {
                num,
                seconds,
                beats,
                name,
                color,
            } => {
                // A loop is measured in beats; without a grid its length is unknown.
                let Some(spb) = song.seconds_per_beat else {
                    continue;
                };
                (num, seconds, Some(beats * spb), name, color)
            }
        };
        if !(1..=SLOTS).contains(&num) {
            continue;
        }
        track.hotcues.push(GuestCue {
            slot: u8::try_from(num - 1).unwrap_or(0),
            seconds,
            loop_seconds,
            color,
            name,
        });
    }
    track.hotcues.sort_by_key(|c| c.slot);
    track
}

/// Maps a playlist path written under another mount (`/Volumes/<name>/…`, `E:/…`) onto the
/// stick: the longest tail of the path that names a file under `root`.
fn on_stick(raw: &Path, root: &Path) -> Option<PathBuf> {
    if raw.starts_with(root) && raw.is_file() {
        return Some(raw.to_path_buf());
    }
    let parts: Vec<String> = raw
        .to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    (0..parts.len())
        .map(|skip| {
            parts[skip..]
                .iter()
                .fold(root.to_path_buf(), |p, part| p.join(part))
        })
        .find(|candidate| {
            candidate.is_file()
                && candidate
                    .components()
                    .all(|c| !matches!(c, Component::ParentDir))
        })
}
