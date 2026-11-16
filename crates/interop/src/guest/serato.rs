//! The Serato branch of a guest stick: `_Serato_/database V2`, `Subcrates` and the cues in
//! each file's `Serato Markers2` tag.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    GuestApp, GuestCue, GuestPlaylist, GuestSource, GuestTrack, dir_on_stick, on_stick as confined,
    track_from_tags,
};
use crate::export::FileKind;
use crate::export::serato::markers::{Marker, read_markers2};
use crate::export::serato::tags::{read_id3, serato_frames};
use crate::export::serato::tlv::{Body, Record, decode, utf16be};
use crate::export::timing::SERATO_INFO_FRAME_FRAMES;

/// Slots of a guest hotcue.
const SLOTS: u8 = 8;
const MARKERS2: &str = "Serato Markers2";

/// Reads the stick's Serato data, or `None` without the `_Serato_` folder.
pub(super) fn read(root: &Path, markers: &library::GuestMarkers) -> Option<GuestSource> {
    if !markers.serato {
        return None;
    }
    let dir = root.join("_Serato_");
    let mut problems = Vec::new();
    let mut source = GuestSource {
        app: GuestApp::Serato,
        tracks: Vec::new(),
        playlists: Vec::new(),
        problem: None,
    };
    let mut index: HashMap<PathBuf, usize> = HashMap::new();
    let mut outside = 0;

    if !dir_on_stick(root, &dir) {
        source.problem = Some("_Serato_ is outside this stick".into());
        return Some(source);
    }
    let database = dir.join("database V2");
    if database.is_file() && confined(root, &database).is_none() {
        problems.push("database V2 is outside this stick".to_owned());
    } else if database.is_file() {
        match fs::read(&database)
            .map_err(|e| e.to_string())
            .and_then(|b| decode(&b))
        {
            Ok(records) => {
                for record in records.iter().filter(|r| &r.tag == b"otrk") {
                    let Body::Records(fields) = &record.body else {
                        continue;
                    };
                    let Some(path) =
                        text(fields, b"pfil").and_then(|p| on_stick(root, &p, &mut outside))
                    else {
                        continue;
                    };
                    if index.contains_key(&path) {
                        continue;
                    }
                    index.insert(path.clone(), source.tracks.len());
                    source.tracks.push(track(path, fields));
                }
            }
            Err(e) => problems.push(format!("database V2: {e}")),
        }
    }

    let crates = dir.join("Subcrates");
    let crates_ok = dir_on_stick(root, &crates);
    if !crates_ok && crates.exists() {
        problems.push("Subcrates is outside this stick".to_owned());
    }
    for (name, file) in crate_files(root, &crates, &dir)
        .into_iter()
        .filter(|_| crates_ok)
    {
        if confined(root, &file).is_none() {
            problems.push(format!("{name}.crate is outside this stick"));
            continue;
        }
        let records = match fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|b| decode(&b))
        {
            Ok(r) => r,
            Err(e) => {
                problems.push(format!("{name}.crate: {e}"));
                continue;
            }
        };
        let mut tracks = Vec::new();
        for record in records.iter().filter(|r| &r.tag == b"otrk") {
            let Body::Records(fields) = &record.body else {
                continue;
            };
            let Some(path) = text(fields, b"ptrk").and_then(|p| on_stick(root, &p, &mut outside))
            else {
                continue;
            };
            let at = *index.entry(path.clone()).or_insert_with(|| {
                source.tracks.push(track_with_cues(track_from_tags(path)));
                source.tracks.len() - 1
            });
            tracks.push(at);
        }
        source.playlists.push(GuestPlaylist {
            name: name.replace("%%", " / "),
            tracks,
        });
    }
    if outside > 0 {
        problems.push(format!(
            "{outside} {} outside this stick",
            if outside == 1 { "path is" } else { "paths are" }
        ));
    }
    if !problems.is_empty() {
        source.problem = Some(problems.join("; "));
    }
    Some(source)
}

/// The text of the first `tag` field among `fields`.
fn text(fields: &[Record], tag: &[u8; 4]) -> Option<String> {
    fields.iter().find_map(|r| match &r.body {
        Body::Bytes(b) if &r.tag == tag => Some(utf16be(b)),
        _ => None,
    })
}

/// A database track: tags the database lacks come from the file.
fn track(path: PathBuf, fields: &[Record]) -> GuestTrack {
    let mut track = track_from_tags(path);
    let field = |tag: &[u8; 4]| text(fields, tag).filter(|t| !t.trim().is_empty());
    if let Some(title) = field(b"tsng") {
        track.title = Some(title);
    }
    if let Some(artist) = field(b"tart") {
        track.artist = Some(artist);
    }
    if let Some(album) = field(b"talb") {
        track.album = Some(album);
    }
    track.bpm = field(b"tbpm")
        .and_then(|b| b.trim().parse::<f64>().ok())
        .filter(|b| *b > 0.0);
    track_with_cues(track)
}

/// `track` with the hotcues of its file's `Serato Markers2` tag.
fn track_with_cues(mut track: GuestTrack) -> GuestTrack {
    track.hotcues = cues_of(&track.path);
    track
}

/// The crate files of `crates` as (stem, path), in `neworder.pref` order and then by name.
fn crate_files(root: &Path, crates: &Path, serato_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(crates) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("crate"))
        })
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let pref = serato_dir.join("neworder.pref");
    let listed = confined(root, &pref)
        .and_then(|p| fs::read(p).ok())
        .map(|b| utf16be(&b))
        .unwrap_or_default();
    let order: Vec<&str> = listed
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|l| l.trim().strip_prefix("[crate]"))
        .collect();
    // A stable sort keeps the by-name order of crates the file does not list.
    files.sort_by_key(|(stem, _)| order.iter().position(|o| o == stem).unwrap_or(order.len()));
    files
}

/// Joins a root-relative path with `/` separators onto `root`, one component at a time;
/// `None` when it climbs out of the stick or names no file; a file that exists but lies
/// outside the stick is counted in `outside`.
fn on_stick(root: &Path, raw: &str, outside: &mut usize) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in raw.replace('\\', "/").split('/').filter(|p| !p.is_empty()) {
        path.push(part);
    }
    let found = confined(root, &path);
    if found.is_none() && path.is_file() {
        *outside += 1;
    }
    found
}

/// The hotcues in `path`'s `Serato Markers2` tag, with an MP3's Info-frame shift removed
/// (the inverse of what export adds).
fn cues_of(path: &Path) -> Vec<GuestCue> {
    let kind = FileKind::of(path);
    let Ok(Some(tag)) = read_id3(path, &kind) else {
        return Vec::new();
    };
    let Some(data) = serato_frames(&tag).remove(MARKERS2) else {
        return Vec::new();
    };
    let shift = if kind == FileKind::Mp3 && decode::lead_in(path).is_ok_and(|l| l.gapless_header) {
        library::tags::read(path)
            .sample_rate
            .filter(|&r| r > 0)
            .map_or(0.0, |r| SERATO_INFO_FRAME_FRAMES / f64::from(r))
    } else {
        0.0
    };
    cues_from_markers(read_markers2(&data), shift)
}

/// Hotcues from `markers`, `shift` seconds earlier. A slot holds one cue: a cue wins over a
/// loop at its index, and the first of any two at one index wins.
fn cues_from_markers(markers: Vec<Marker>, shift: f64) -> Vec<GuestCue> {
    let seconds = |ms: u32| (f64::from(ms) / 1000.0 - shift).max(0.0);
    let named = |name: String| Some(name).filter(|n| !n.is_empty());
    let mut cues: Vec<GuestCue> = markers
        .into_iter()
        .filter_map(|marker| match marker {
            Marker::Cue {
                index,
                ms,
                rgb,
                name,
            } if index < SLOTS => Some(GuestCue {
                slot: index,
                seconds: seconds(ms),
                loop_seconds: None,
                color: Some(rgb),
                name: named(name),
            }),
            Marker::Loop {
                index,
                start_ms,
                end_ms,
                name,
            } if index < SLOTS && end_ms > start_ms => Some(GuestCue {
                slot: index,
                seconds: seconds(start_ms),
                loop_seconds: Some(seconds(end_ms) - seconds(start_ms)),
                color: None,
                name: named(name),
            }),
            _ => None,
        })
        .collect();
    // Cues first, so a loop at a cue's index is the one dropped.
    cues.sort_by_key(|c| (c.slot, c.loop_seconds.is_some()));
    cues.dedup_by_key(|c| c.slot);
    cues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serato_loop_never_takes_a_cue_slot() {
        let markers = vec![
            Marker::Loop {
                index: 1,
                start_ms: 5000,
                end_ms: 6000,
                name: String::new(),
            },
            Marker::Cue {
                index: 1,
                ms: 1000,
                rgb: 0xCC8800,
                name: String::new(),
            },
            Marker::Loop {
                index: 2,
                start_ms: 7000,
                end_ms: 9000,
                name: String::new(),
            },
        ];
        let cues = cues_from_markers(markers, 0.0);
        let slots: Vec<u8> = cues.iter().map(|c| c.slot).collect();
        assert_eq!(slots, [1, 2]);
        assert_eq!(cues[0].loop_seconds, None);
        assert!((cues[0].seconds - 1.0).abs() < 1e-9);
        assert_eq!(cues[1].loop_seconds, Some(2.0));
    }
}
