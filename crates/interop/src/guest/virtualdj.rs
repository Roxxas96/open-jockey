//! The VirtualDJ branch of a guest stick: `VirtualDJ/database.xml` and `VirtualDJ/MyLists`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{
    GuestApp, GuestCue, GuestPlaylist, GuestSource, GuestTrack, MAX_DATABASE_BYTES, MAX_LIST_BYTES,
    MAX_PATH_PARTS, dir_on_stick, on_stick as confined, track_from_tags,
};
use crate::virtualdj::{VdjPoi, VdjSong, read_database_capped, read_playlists_capped};

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

    if !dir_on_stick(root, &dir) {
        source.problem = Some("VirtualDJ is outside this stick".into());
        return Some(source);
    }
    if database.is_file() && confined(root, &database).is_none() {
        problems.push("database.xml is outside this stick".to_owned());
    } else if database.is_file() {
        match read_database_capped(&database, Some(root), Some(MAX_DATABASE_BYTES)) {
            Ok(songs) => {
                let mut outside = 0;
                for song in songs {
                    if confined(root, &song.path).is_none() {
                        if song.path.is_file() {
                            outside += 1;
                        }
                        continue;
                    }
                    index.insert(song.path.clone(), source.tracks.len());
                    source.tracks.push(track(song));
                }
                if outside > 0 {
                    problems.push(format!(
                        "{outside} database {} outside this stick",
                        if outside == 1 { "song is" } else { "songs are" }
                    ));
                }
            }
            Err(e) => problems.push(e.to_string()),
        }
    }

    let lists = dir.join("MyLists");
    if lists.is_dir() && !dir_on_stick(root, &lists) {
        problems.push("MyLists is outside this stick".to_owned());
    } else if lists.is_dir() {
        let order = lists.join("order");
        let order = confined(root, &order);
        match read_playlists_capped(&[lists], order.as_deref(), Some(MAX_LIST_BYTES)) {
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
    // A slot holds one cue: the first in the database wins, as with the other apps.
    track.hotcues.sort_by_key(|c| c.slot);
    track.hotcues.dedup_by_key(|c| c.slot);
    track
}

/// Maps a playlist path written under another mount (`/Volumes/<name>/…`, `E:/…`) onto the
/// stick: the longest tail of the path that names a file under `root`. A path of more than
/// [`MAX_PATH_PARTS`] parts is refused unread, so the tails tried stay few and short.
fn on_stick(raw: &Path, root: &Path) -> Option<PathBuf> {
    if raw.starts_with(root) {
        return confined(root, raw);
    }
    let parts: Vec<String> = raw
        .to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.len() > MAX_PATH_PARTS {
        return None;
    }
    (0..parts.len())
        .map(|skip| {
            parts[skip..]
                .iter()
                .fold(root.to_path_buf(), |p, part| p.join(part))
        })
        .find_map(|candidate| confined(root, &candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_cue_of_a_slot_is_kept() {
        let cue = |num, seconds, name: &str| VdjPoi::Hotcue {
            num,
            seconds,
            name: Some(name.into()),
            color: None,
        };
        let song = VdjSong {
            path: PathBuf::from("missing.mp3"),
            size: None,
            title: Some("t".into()),
            artist: None,
            album: None,
            seconds_per_beat: Some(0.5),
            pois: vec![
                cue(2, 4.0, "first"),
                VdjPoi::Loop {
                    num: 2,
                    seconds: 8.0,
                    beats: 4.0,
                    name: Some("second".into()),
                    color: None,
                },
                cue(1, 1.0, "one"),
                cue(2, 9.0, "third"),
            ],
            other_markers: 0,
            from_drive: true,
        };
        let cues = track(song).hotcues;
        let got: Vec<(u8, &str)> = cues
            .iter()
            .map(|c| (c.slot, c.name.as_deref().unwrap()))
            .collect();
        assert_eq!(got, [(0, "one"), (1, "first")]);
    }
}
