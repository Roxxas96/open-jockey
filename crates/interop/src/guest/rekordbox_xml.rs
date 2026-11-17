//! The rekordbox XML branch of a guest stick: a `rekordbox.xml` export at the stick's root.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::{
    GuestApp, GuestCue, GuestPlaylist, GuestSource, GuestTrack, on_stick, track_from_tags,
};

/// Slots of a guest hotcue; rekordbox's `Num` 0..=7.
const SLOTS: i32 = 8;
/// How many folder levels below the stick root a track is looked for by its file name.
const SEARCH_DEPTH: usize = 6;
/// How many files the by-name search remembers, so a huge stick cannot make a read unbounded.
const SEARCH_LIMIT: usize = 200_000;
/// The largest `rekordbox.xml` read.
const MAX_XML_BYTES: u64 = 256 * 1024 * 1024;

/// Reads the stick's `rekordbox.xml`, or `None` without the marker.
pub(super) fn read(root: &Path, markers: &library::GuestMarkers) -> Option<GuestSource> {
    if !markers.rekordbox_xml {
        return None;
    }
    let mut source = GuestSource {
        app: GuestApp::RekordboxXml,
        tracks: Vec::new(),
        playlists: Vec::new(),
        problem: None,
    };
    let file = root.join("rekordbox.xml");
    let Some(file) = on_stick(root, &file) else {
        source.problem = Some("rekordbox.xml is outside this stick".into());
        return Some(source);
    };
    let bytes = match std::fs::metadata(&file) {
        Ok(m) if m.len() > MAX_XML_BYTES => {
            source.problem = Some("rekordbox.xml is too large to read".into());
            return Some(source);
        }
        Ok(_) => std::fs::read(&file),
        Err(e) => Err(e),
    };
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) => {
            source.problem = Some(format!("rekordbox.xml could not be read: {e}"));
            return Some(source);
        }
    };
    let mut locator = Locator::new(root);
    let mut problems = Vec::new();
    if let Err(why) = parse(&bytes, &mut locator, &mut source) {
        problems.push(format!("rekordbox.xml is damaged: {why}"));
    }
    problems.extend(locator.problems());
    if !problems.is_empty() {
        source.problem = Some(problems.join("; "));
    }
    Some(source)
}

/// One `<TRACK>` of the collection while it is read.
#[derive(Default)]
struct Pending {
    id: String,
    name: String,
    artist: String,
    album: String,
    location: String,
    bpm: Option<f64>,
    first_beat: Option<f64>,
    main_cue: Option<f64>,
    hotcues: Vec<GuestCue>,
}

/// Streams the document: the collection's tracks, then the playlist nodes in order.
fn parse(bytes: &[u8], locator: &mut Locator, source: &mut GuestSource) -> Result<(), String> {
    let mut reader = Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut in_collection = false;
    let mut in_playlists = false;
    let mut track: Option<Pending> = None;
    // Open `<NODE>`s of the playlist tree: a name for a folder, and whether it is a playlist.
    let mut nodes: Vec<(String, bool)> = Vec::new();
    let mut open_playlist: Option<GuestPlaylist> = None;
    let mut depth = 0usize;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("at byte {}: {e}", reader.error_position()))?;
        let (e, closes) = match &event {
            Event::Start(e) => {
                depth += 1;
                (Some(e), false)
            }
            Event::Empty(e) => (Some(e), true),
            _ => (None, false),
        };
        if let Some(e) = e {
            match e.name().as_ref() {
                "COLLECTION" => in_collection = true,
                "PLAYLISTS" => in_playlists = true,
                "TRACK" if in_collection => {
                    let pending = Pending {
                        id: attr(e, "TrackID").unwrap_or_default(),
                        name: attr(e, "Name").unwrap_or_default(),
                        artist: attr(e, "Artist").unwrap_or_default(),
                        album: attr(e, "Album").unwrap_or_default(),
                        location: attr(e, "Location").unwrap_or_default(),
                        ..Pending::default()
                    };
                    if closes {
                        finish(pending, locator, source, &mut ids);
                    } else {
                        track = Some(pending);
                    }
                }
                "TEMPO" => {
                    if let Some(t) = track.as_mut().filter(|t| t.bpm.is_none()) {
                        t.bpm = number(e, "Bpm").filter(|b| *b > 0.0);
                        t.first_beat = number(e, "Inizio");
                    }
                }
                "POSITION_MARK" => {
                    if let Some(t) = track.as_mut() {
                        mark(e, t);
                    }
                }
                "NODE" if in_playlists => {
                    let name = attr(e, "Name").unwrap_or_default();
                    let is_list = attr(e, "Type").as_deref() == Some("1");
                    if is_list {
                        // A playlist inside a playlist: the outer one is complete as read.
                        source.playlists.extend(open_playlist.take());
                        open_playlist = Some(GuestPlaylist {
                            name: flatten(&nodes, &name),
                            tracks: Vec::new(),
                        });
                    }
                    if closes {
                        source.playlists.extend(open_playlist.take());
                    } else {
                        nodes.push((name, is_list));
                    }
                }
                "TRACK" if in_playlists => {
                    let at = attr(e, "Key").and_then(|k| ids.get(&k).copied());
                    if let (Some(list), Some(at)) = (open_playlist.as_mut(), at) {
                        list.tracks.push(at);
                    }
                }
                _ => {}
            }
        } else if let Event::End(e) = &event {
            depth = depth.saturating_sub(1);
            match e.name().as_ref() {
                "COLLECTION" => in_collection = false,
                "PLAYLISTS" => in_playlists = false,
                "TRACK" if in_collection => {
                    if let Some(t) = track.take() {
                        finish(t, locator, source, &mut ids);
                    }
                }
                // The pop runs in the guard so a folder closes too; only a playlist is stored.
                "NODE" if in_playlists && nodes.pop().is_some_and(|(_, is_list)| is_list) => {
                    source.playlists.extend(open_playlist.take());
                }
                _ => {}
            }
        } else if matches!(event, Event::Eof) {
            if depth > 0 {
                return Err("the file is truncated".into());
            }
            break;
        }
        buf.clear();
    }
    Ok(())
}

/// `Folder / Name`: the folders above a playlist, leaving out the tree's root node.
fn flatten(open: &[(String, bool)], name: &str) -> String {
    open.iter()
        .skip(1)
        .map(|(n, _)| n.as_str())
        .chain([name])
        .collect::<Vec<_>>()
        .join(" / ")
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| {
            a.normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

fn number(e: &BytesStart<'_>, key: &str) -> Option<f64> {
    attr(e, key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// Adds one `<POSITION_MARK>` to the track: a hotcue, a saved loop or a memory cue.
fn mark(e: &BytesStart<'_>, t: &mut Pending) {
    let (Some(num), Some(start)) = (number(e, "Num"), number(e, "Start")) else {
        return;
    };
    if start < 0.0 {
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let num = num as i32;
    if num == -1 {
        // Only a plain memory cue; fades, load markers and memory loops are not cues.
        if attr(e, "Type").as_deref() == Some("0") {
            t.main_cue = t.main_cue.or(Some(start));
        }
        return;
    }
    if !(0..SLOTS).contains(&num) {
        return;
    }
    let slot = u8::try_from(num).unwrap_or(0);
    if t.hotcues.iter().any(|c| c.slot == slot) {
        return;
    }
    let loop_seconds = match attr(e, "Type").as_deref() {
        Some("0") => None,
        Some("4") => match number(e, "End") {
            Some(end) if end > start => Some(end - start),
            _ => None,
        },
        _ => return,
    };
    let channel = |key| number(e, key).map(|v| v.clamp(0.0, 255.0) as u32);
    let color = match (channel("Red"), channel("Green"), channel("Blue")) {
        (Some(r), Some(g), Some(b)) => Some(r << 16 | g << 8 | b),
        _ => None,
    };
    t.hotcues.push(GuestCue {
        slot,
        seconds: start,
        loop_seconds,
        color,
        name: attr(e, "Name").filter(|n| !n.is_empty()),
    });
}

/// Resolves a finished collection track onto the stick, or drops it.
fn finish(
    t: Pending,
    locator: &mut Locator,
    source: &mut GuestSource,
    ids: &mut HashMap<String, usize>,
) {
    let Some(path) = locator.find(&t.location) else {
        return;
    };
    let mut track: GuestTrack = track_from_tags(path);
    if !t.name.is_empty() {
        track.title = Some(t.name);
    }
    if !t.artist.is_empty() {
        track.artist = Some(t.artist);
    }
    if !t.album.is_empty() {
        track.album = Some(t.album);
    }
    track.bpm = t.bpm;
    track.first_beat = t.first_beat;
    track.main_cue = t.main_cue;
    track.hotcues = t.hotcues;
    track.hotcues.sort_by_key(|c| c.slot);
    ids.insert(t.id, source.tracks.len());
    source.tracks.push(track);
}

/// Maps a `Location` onto a file of the stick, and counts what it had to refuse or guess.
struct Locator {
    root: PathBuf,
    /// The stick's own folder name, as it appears in a path written while it was mounted.
    stick_name: Option<OsString>,
    /// Files by name, built on first use by one bounded walk.
    by_name: Option<HashMap<OsString, Vec<PathBuf>>>,
    outside: usize,
    missing: usize,
    ambiguous: usize,
    /// Whether the by-name walk stopped at its file cap.
    capped: bool,
    limit: usize,
}

impl Locator {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            stick_name: root.file_name().map(ToOwned::to_owned),
            by_name: None,
            outside: 0,
            missing: 0,
            ambiguous: 0,
            capped: false,
            limit: SEARCH_LIMIT,
        }
    }

    fn problems(&self) -> Vec<String> {
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                one.to_owned()
            } else {
                many.to_owned()
            }
        };
        let mut out = Vec::new();
        if self.outside > 0 {
            out.push(format!(
                "{} {} outside this stick",
                self.outside,
                plural(self.outside, "track location is", "track locations are")
            ));
        }
        if self.missing > 0 {
            out.push(format!(
                "{} {} not found on this stick",
                self.missing,
                plural(self.missing, "track was", "tracks were")
            ));
        }
        if self.ambiguous > 0 {
            out.push(format!(
                "{} {} several files on this stick; the one under Contents or else the shortest path was used",
                self.ambiguous,
                plural(self.ambiguous, "track name matches", "track names match")
            ));
        }
        if self.capped {
            out.push("this stick was too large to search fully for tracks by name".into());
        }
        out
    }

    /// The file on the stick for a `file://` location, counting a refusal or a miss.
    fn find(&mut self, location: &str) -> Option<PathBuf> {
        let parts = parts_of(location);
        if parts.is_empty() || parts.iter().any(|p| p == "..") {
            self.outside += 1;
            return None;
        }
        // The path as written, when it already lies on this stick.
        let written: PathBuf = parts.iter().collect::<PathBuf>();
        let written = Path::new("/").join(written);
        if written.starts_with(&self.root)
            && let Some(path) = on_stick(&self.root, &written)
        {
            return Some(path);
        }
        // The path after the stick's name, then its longer tails, then the bare file name.
        let after_name = self
            .stick_name
            .as_ref()
            .and_then(|n| parts.iter().position(|p| OsString::from(p) == *n))
            .map(|i| i + 1);
        let tails = after_name
            .into_iter()
            .chain(1..parts.len().saturating_sub(1))
            .filter(|&skip| skip < parts.len().saturating_sub(1));
        for skip in tails {
            let candidate = parts[skip..]
                .iter()
                .fold(self.root.clone(), |p, s| p.join(s));
            if let Some(path) = on_stick(&self.root, &candidate) {
                return Some(path);
            }
        }
        let name = OsString::from(parts.last()?);
        let found = self.index().get(&name).cloned().unwrap_or_default();
        let (best, decisive) = choose(&self.root, found);
        if !decisive {
            self.ambiguous += 1;
        }
        let path = best.and_then(|p| on_stick(&self.root, &p));
        if path.is_none() {
            self.missing += 1;
        }
        path
    }

    fn index(&mut self) -> &HashMap<OsString, Vec<PathBuf>> {
        let (root, limit) = (&self.root, self.limit);
        let capped = &mut self.capped;
        self.by_name.get_or_insert_with(|| {
            let mut map = HashMap::new();
            let mut seen = 0;
            *capped = !walk(root, 0, (&mut seen, limit), &mut map);
            map
        })
    }
}

/// Indexes the files under `dir` by name: no hidden folders, no links, a bounded depth.
/// `budget` is the files seen so far and the most to remember; returns `false` when the walk
/// stopped at that cap.
fn walk(
    dir: &Path,
    depth: usize,
    budget: (&mut usize, usize),
    map: &mut HashMap<OsString, Vec<PathBuf>>,
) -> bool {
    let (seen, limit) = budget;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        if *seen >= limit {
            return false;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        if kind.is_dir() {
            if depth < SEARCH_DEPTH && !name.to_string_lossy().starts_with('.') {
                subdirs.push(entry.path());
            }
        } else if kind.is_file() {
            *seen += 1;
            map.entry(name).or_default().push(entry.path());
        }
    }
    subdirs.sort();
    for sub in subdirs {
        if !walk(&sub, depth + 1, (&mut *seen, limit), map) {
            return false;
        }
    }
    true
}

/// The best of the files sharing a name, and whether the choice was decisive: files under
/// `Contents/` win, then the shortest path; a tie is broken by path order and not decisive.
fn choose(root: &Path, mut found: Vec<PathBuf>) -> (Option<PathBuf>, bool) {
    let rank = |p: &PathBuf| {
        let rel = p.strip_prefix(root).unwrap_or(p);
        let in_contents = rel
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "Contents");
        (!in_contents, rel.components().count())
    };
    found.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    let decisive = found.len() < 2 || rank(&found[0]) != rank(&found[1]);
    (found.into_iter().next(), decisive)
}

/// The path components of a `file://` location, percent-decoded, without a host or drive.
fn parts_of(location: &str) -> Vec<String> {
    let rest = location
        .strip_prefix("file://localhost")
        .or_else(|| location.strip_prefix("file://"))
        .unwrap_or(location);
    let decoded = percent_decode_str(rest)
        .decode_utf8_lossy()
        .replace('\\', "/");
    let mut parts: Vec<String> = decoded
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .map(str::to_owned)
        .collect();
    if parts
        .first()
        .is_some_and(|p| p.len() == 2 && p.ends_with(':'))
    {
        parts.remove(0);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_search_stops_at_its_cap_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..5 {
            std::fs::write(dir.path().join(format!("{n}.mp3")), b"x").unwrap();
        }
        let mut locator = Locator::new(dir.path());
        locator.limit = 3;
        assert_eq!(locator.find("file://localhost/Elsewhere/none.mp3"), None);
        assert!(locator.problems().iter().any(|p| p.contains("too large")));
    }
}
