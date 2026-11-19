//! The Serato branch of a guest stick: `_Serato_/database V2`, `Subcrates`, and the cues and
//! grid in each file's `Serato Markers2` and `Serato Tempo Map` (or older `Serato BeatGrid`)
//! tags.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    GuestApp, GuestCue, GuestPlaylist, GuestSource, GuestTrack, MAX_DATABASE_BYTES, MAX_LIST_BYTES,
    dir_on_stick, on_stick as confined, read_capped, track_from_tags,
};
use crate::export::FileKind;
use crate::export::serato::markers::{Marker, read_markers2};
use crate::export::serato::tags::{read_id3, serato_frames};
use crate::export::serato::tlv::{Body, Record, decode, utf16be};
use crate::export::timing::{App, app_offset_seconds};

/// Slots of a guest hotcue.
const SLOTS: u8 = 8;
const MARKERS2: &str = "Serato Markers2";
const TEMPO_MAP: &str = "Serato Tempo Map";
const BEATGRID: &str = "Serato BeatGrid";

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
    let mut paths = Paths::default();

    if !dir_on_stick(root, &dir) {
        source.problem = Some("_Serato_ is outside this stick".into());
        return Some(source);
    }
    let database = dir.join("database V2");
    if database.is_file() && confined(root, &database).is_none() {
        problems.push("database V2 is outside this stick".to_owned());
    } else if database.is_file() {
        match read_capped(&database, MAX_DATABASE_BYTES)
            .map_err(|e| format!("database V2 {e}"))
            .and_then(|b| decode(&b).map_err(|e| format!("database V2: {e}")))
        {
            Ok(records) => {
                for record in records.iter().filter(|r| &r.tag == b"otrk") {
                    let Body::Records(fields) = &record.body else {
                        continue;
                    };
                    let Some(path) = text(fields, b"pfil")
                        .and_then(|p| on_stick(root, &p, &mut paths, Entry::Database))
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
            Err(e) => problems.push(e),
        }
    }

    let crates = dir.join("Subcrates");
    let crates_ok = dir_on_stick(root, &crates);
    if !crates_ok && crates.exists() {
        problems.push("Subcrates is outside this stick".to_owned());
    }
    for (name, file) in crate_files(root, &crates, &dir, &mut problems)
        .into_iter()
        .filter(|_| crates_ok)
    {
        if confined(root, &file).is_none() {
            problems.push(format!("{name}.crate is outside this stick"));
            continue;
        }
        let records = match read_capped(&file, MAX_LIST_BYTES)
            .map_err(|e| format!("{name}.crate {e}"))
            .and_then(|b| decode(&b).map_err(|e| format!("{name}.crate: {e}")))
        {
            Ok(r) => r,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let mut tracks = Vec::new();
        for record in records.iter().filter(|r| &r.tag == b"otrk") {
            let Body::Records(fields) = &record.body else {
                continue;
            };
            let Some(path) =
                text(fields, b"ptrk").and_then(|p| on_stick(root, &p, &mut paths, Entry::Crate))
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
    problems.extend(paths.problems());
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
        .filter(|b| b.is_finite() && *b > 0.0);
    track_with_cues(track)
}

/// `track` with the hotcues of its file's `Serato Markers2` tag and, when the file has one,
/// the grid of its `Serato Tempo Map` or `Serato BeatGrid` tag (whose BPM replaces the
/// database's, so the two belong to one grid).
fn track_with_cues(mut track: GuestTrack) -> GuestTrack {
    let (cues, grid) = tags_of(&track.path);
    track.hotcues = cues;
    if let Some((first_beat, bpm)) = grid {
        track.first_beat = Some(first_beat);
        track.bpm = Some(bpm);
    }
    track
}

/// The crate files of `crates` as (stem, path), in `neworder.pref` order and then by name. A
/// `neworder.pref` that cannot be read leaves the by-name order and adds a problem.
fn crate_files(
    root: &Path,
    crates: &Path,
    serato_dir: &Path,
    problems: &mut Vec<String>,
) -> Vec<(String, PathBuf)> {
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
    let listed = match confined(root, &pref).map(|p| read_capped(&p, MAX_LIST_BYTES)) {
        Some(Ok(bytes)) => utf16be(&bytes),
        Some(Err(e)) => {
            problems.push(format!("neworder.pref {e}"));
            String::new()
        }
        None => String::new(),
    };
    let order: Vec<&str> = listed
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|l| l.trim().strip_prefix("[crate]"))
        .collect();
    // A stable sort keeps the by-name order of crates the file does not list.
    files.sort_by_key(|(stem, _)| order.iter().position(|o| o == stem).unwrap_or(order.len()));
    files
}

/// Where a path in Serato's files was read from, for the count of missing files.
#[derive(Clone, Copy)]
enum Entry {
    Database,
    Crate,
}

/// The paths a read refused or did not find.
#[derive(Default)]
struct Paths {
    outside: usize,
    missing_tracks: usize,
    missing_entries: usize,
}

impl Paths {
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let n = self.outside;
        if n > 0 {
            let what = if n == 1 { "path is" } else { "paths are" };
            out.push(format!("{n} {what} outside this stick"));
        }
        let n = self.missing_tracks;
        if n > 0 {
            let what = if n == 1 { "track is" } else { "tracks are" };
            out.push(format!("{n} database V2 {what} not on this stick"));
        }
        let n = self.missing_entries;
        if n > 0 {
            let what = if n == 1 {
                "entry points"
            } else {
                "entries point"
            };
            out.push(format!(
                "{n} crate {what} at files that are not on this stick"
            ));
        }
        out
    }
}

/// Joins a root-relative path with `/` separators onto `root`, one component at a time;
/// `None` when it climbs out of the stick or names no file. A `..` path and a file that exists
/// but lies outside the stick are counted as outside; any other miss as missing.
fn on_stick(root: &Path, raw: &str, paths: &mut Paths, entry: Entry) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let mut climbs = false;
    for part in raw.replace('\\', "/").split('/').filter(|p| !p.is_empty()) {
        climbs |= part == "..";
        path.push(part);
    }
    let found = confined(root, &path);
    if found.is_none() {
        if climbs || path.is_file() {
            paths.outside += 1;
        } else {
            match entry {
                Entry::Database => paths.missing_tracks += 1,
                Entry::Crate => paths.missing_entries += 1,
            }
        }
    }
    found
}

/// The hotcues in `path`'s `Serato Markers2` tag and the grid (first beat in seconds, BPM)
/// of its `Serato Tempo Map`, else of its `Serato BeatGrid`, with an MP3's Info-frame shift
/// removed from both (the inverse of what export adds).
fn tags_of(path: &Path) -> (Vec<GuestCue>, Option<(f64, f64)>) {
    let kind = FileKind::of(path);
    let Ok(Some(tag)) = read_id3(path, &kind) else {
        return (Vec::new(), None);
    };
    let mut frames = serato_frames(&tag);
    let markers = frames.remove(MARKERS2);
    let grid = frames
        .get(TEMPO_MAP)
        .and_then(|d| tempo_map(d))
        .or_else(|| frames.get(BEATGRID).and_then(|d| beatgrid(d)));
    if markers.is_none() && grid.is_none() {
        return (Vec::new(), None);
    }
    let shift = match (decode::lead_in(path), library::tags::read(path).sample_rate) {
        (Ok(lead_in), Some(rate)) => app_offset_seconds(App::Serato, kind, &lead_in, rate),
        _ => 0.0,
    };
    let cues = markers.map_or_else(Vec::new, |data| {
        cues_from_markers(read_markers2(&data), shift)
    });
    (cues, grid.map(|(first, bpm)| (first - shift, bpm)))
}

/// The first beat (seconds) and BPM of a `Serato Tempo Map` frame, as Serato DJ writes it
/// (version 2): the first segment's beat position, a big-endian `f64` at byte 4, and its BPM,
/// an `f64` at byte 24. The layout is read off the fixture's files (see its README), not a
/// published format; anything else gives `None`.
fn tempo_map(data: &[u8]) -> Option<(f64, f64)> {
    if data.first() != Some(&2) {
        return None;
    }
    let f64_at = |at: usize| {
        let bytes: [u8; 8] = data.get(at..at + 8)?.try_into().ok()?;
        Some(f64::from_be_bytes(bytes))
    };
    let (first, bpm) = (f64_at(4)?, f64_at(24)?);
    (first.is_finite() && bpm.is_finite() && bpm > 0.0).then_some((first, bpm))
}

/// The first beat (seconds) and first segment's BPM of a `Serato BeatGrid` frame: `01 00`, a
/// `u32` marker count, then 8-byte markers of an `f32` position and either the beats to the
/// next marker or, on the last one, an `f32` BPM. `None` when damaged.
fn beatgrid(data: &[u8]) -> Option<(f64, f64)> {
    if data.get(..2)? != [1, 0] {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let f32_at = |at: usize| u32_at(at).map(|b| f64::from(f32::from_bits(b)));
    let count = u32_at(2)?;
    let first = f32_at(6)?;
    let bpm = if count == 1 {
        f32_at(10)?
    } else {
        let beats = f64::from(u32_at(10)?);
        let span = f32_at(14)? - first;
        if count == 0 || span <= 0.0 {
            return None;
        }
        beats * 60.0 / span
    };
    (first.is_finite() && bpm.is_finite() && bpm > 0.0).then_some((first, bpm))
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
