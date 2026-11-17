//! The rekordbox device branch of a guest stick: `PIONEER/rekordbox/export.pdb` and the
//! `PIONEER/USBANLZ` analysis files its tracks point to. `exportLibrary.db` (the newer
//! encrypted library) and `exportExt.pdb` are never read.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use super::anlz::{AnlzData, read_anlz};
use super::pdb::{NodeRow, PdbLibrary, read_pdb};
use super::{
    GuestApp, GuestPlaylist, GuestSource, GuestTrack, MAX_DEPTH, MAX_NAME_BYTES, MAX_PLAYLISTS,
    dir_on_stick, join_name, on_stick, read_capped, track_from_tags,
};

/// The largest `export.pdb` read.
const MAX_PDB_BYTES: u64 = 512 * 1024 * 1024;

/// Reads the stick's device library, or `None` without the marker.
pub(super) fn read(root: &Path, markers: &library::GuestMarkers) -> Option<GuestSource> {
    if !markers.rekordbox_device {
        return None;
    }
    let mut source = GuestSource {
        app: GuestApp::RekordboxDevice,
        tracks: Vec::new(),
        playlists: Vec::new(),
        problem: None,
    };
    let pioneer = root.join("PIONEER");
    let pdb = pioneer.join("rekordbox").join("export.pdb");
    if !dir_on_stick(root, &pioneer.join("rekordbox")) {
        source.problem = Some("PIONEER/rekordbox is outside this stick".into());
        return Some(source);
    }
    let Some(pdb) = on_stick(root, &pdb) else {
        source.problem = Some("export.pdb is outside this stick".into());
        return Some(source);
    };
    let bytes = match read_capped(&pdb, MAX_PDB_BYTES) {
        Ok(b) => b,
        Err(e) => {
            source.problem = Some(format!("export.pdb {e}"));
            return Some(source);
        }
    };
    let library = match read_pdb(&bytes) {
        Ok(library) => library,
        Err(why) => {
            source.problem = Some(format!("export.pdb is damaged: {why}"));
            return Some(source);
        }
    };
    let mut counts = Counts::default();
    let mut ids: HashMap<u32, usize> = HashMap::new();
    for row in &library.tracks {
        let Some(path) = stick_path(root, &row.file_path).and_then(|p| on_stick(root, &p)) else {
            counts.tracks_refused += 1;
            continue;
        };
        let mut track: GuestTrack = track_from_tags(path);
        if !row.title.is_empty() {
            track.title = Some(row.title.clone());
        }
        track.artist = library
            .artists
            .get(&row.artist_id)
            .cloned()
            .or(track.artist);
        track.album = library.albums.get(&row.album_id).cloned().or(track.album);
        if row.tempo > 0 {
            track.bpm = Some(f64::from(row.tempo) / 100.0);
        }
        match analysis(root, &row.analyze_path) {
            Ok(Some((data, ext_refused))) => {
                counts.anlz_refused += usize::from(ext_refused);
                if let Some((first_beat, bpm)) = data.grid {
                    track.first_beat = Some(first_beat);
                    track.bpm = Some(bpm);
                }
                track.hotcues = data.cues;
                track.main_cue = data.main_cue;
            }
            Ok(None) => {}
            Err(Refused::Outside) => counts.anlz_refused += 1,
            Err(Refused::Missing) => counts.anlz_missing += 1,
            Err(Refused::Unreadable) => counts.anlz_unreadable += 1,
        }
        ids.insert(row.id, source.tracks.len());
        source.tracks.push(track);
    }
    let (lists, too_many) = playlists(&library, &ids);
    source.playlists = lists;
    counts.too_many_playlists = too_many;
    counts.truncated = library.truncated;
    source.problem = counts.problem();
    Some(source)
}

/// What a read had to refuse or could not read.
#[derive(Default)]
struct Counts {
    tracks_refused: usize,
    anlz_refused: usize,
    anlz_missing: usize,
    anlz_unreadable: usize,
    /// The pdb held more rows than are read.
    truncated: bool,
    /// A cap stopped the playlists.
    too_many_playlists: bool,
}

impl Counts {
    fn problem(&self) -> Option<String> {
        let mut out = Vec::new();
        if self.tracks_refused > 0 {
            out.push(format!(
                "{} track(s) are missing or outside this stick",
                self.tracks_refused
            ));
        }
        if self.anlz_refused > 0 {
            out.push(format!(
                "{} analysis file(s) are outside this stick",
                self.anlz_refused
            ));
        }
        if self.anlz_missing > 0 {
            out.push(format!(
                "{} analysis file(s) are missing from this stick",
                self.anlz_missing
            ));
        }
        if self.anlz_unreadable > 0 {
            out.push(format!(
                "{} analysis file(s) could not be read",
                self.anlz_unreadable
            ));
        }
        if self.truncated {
            out.push("export.pdb holds more rows than are read".into());
        }
        if self.too_many_playlists {
            out.push("this stick has too many playlists to show".into());
        }
        (!out.is_empty()).then(|| out.join("; "))
    }
}

enum Refused {
    Outside,
    /// Named by the pdb but not on the stick (a partly copied `USBANLZ`).
    Missing,
    Unreadable,
}

/// The analysis of a track from its `.DAT` path (and the `.EXT` beside it), if it has one.
fn analysis(root: &Path, analyze_path: &str) -> Result<Option<(AnlzData, bool)>, Refused> {
    if analyze_path.is_empty() {
        return Ok(None);
    }
    let dat = stick_path(root, analyze_path).ok_or(Refused::Outside)?;
    let dat = match on_stick(root, &dat) {
        Some(dat) => dat,
        // Neither a file nor a link: nothing is there to be outside the stick.
        None if dat.symlink_metadata().is_err() => return Err(Refused::Missing),
        None => return Err(Refused::Outside),
    };
    // An `.EXT` that is there but not on the stick is refused, not silently left out.
    let ext_path = dat.with_extension("EXT");
    let ext = on_stick(root, &ext_path);
    let ext_refused = ext.is_none() && ext_path.symlink_metadata().is_ok();
    read_anlz(&dat, ext.as_deref())
        .map(|data| Some((data, ext_refused)))
        .map_err(|_| Refused::Unreadable)
}

/// A path the pdb stores (`/Contents/...`, from the stick's root) as a path on the stick;
/// `None` when it holds a `..` or a drive.
fn stick_path(root: &Path, stored: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    let mut any = false;
    for part in stored.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        let part = Path::new(part);
        if !matches!(part.components().next(), Some(Component::Normal(_)))
            || part.components().count() != 1
        {
            return None;
        }
        out.push(part);
        any = true;
    }
    any.then_some(out)
}

/// The playlists of the tree in tree order (each folder's children by `sort_order`, folders
/// before what follows them), each named `Folder / Playlist`, with their tracks in order. The
/// flag is set when a cap stopped the build.
fn playlists(library: &PdbLibrary, ids: &HashMap<u32, usize>) -> (Vec<GuestPlaylist>, bool) {
    let known: HashSet<u32> = library.nodes.iter().map(|n| n.id).collect();
    // Children by parent; a node whose parent is not in the tree hangs off the root (0).
    let mut children: HashMap<u32, Vec<&NodeRow>> = HashMap::new();
    for node in &library.nodes {
        let parent = if known.contains(&node.parent) && node.parent != node.id {
            node.parent
        } else {
            0
        };
        children.entry(parent).or_default().push(node);
    }
    for list in children.values_mut() {
        list.sort_by_key(|n| (n.sort_order, n.id));
    }
    let mut entries: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for &(list, index, track) in &library.entries {
        entries.entry(list).or_default().push((index, track));
    }
    let mut lists = Vec::new();
    let mut name_bytes = 0usize;
    let mut capped = false;
    let mut visited = HashSet::new();
    // Depth-first with an explicit stack: a node still to visit, its depth and the flattened
    // name of the folders above it (built once per folder and shared by its children).
    let mut stack: Vec<(&NodeRow, usize, Rc<str>)> = children
        .get(&0)
        .into_iter()
        .flatten()
        .rev()
        .map(|&n| (n, 0, Rc::from("")))
        .collect();
    while let Some((node, depth, prefix)) = stack.pop() {
        if !visited.insert(node.id) {
            continue;
        }
        if node.is_playlist {
            let name = join_name(&prefix, &node.name);
            name_bytes += name.len();
            if lists.len() >= MAX_PLAYLISTS || name_bytes > MAX_NAME_BYTES {
                capped = true;
                break;
            }
            let mut rows = entries.remove(&node.id).unwrap_or_default();
            rows.sort_by_key(|&(index, _)| index);
            lists.push(GuestPlaylist {
                name,
                tracks: rows
                    .into_iter()
                    .filter_map(|(_, track)| ids.get(&track).copied())
                    .collect(),
            });
        } else if depth + 1 < MAX_DEPTH {
            let below: Rc<str> = Rc::from(join_name(&prefix, &node.name));
            for &child in children.get(&node.id).into_iter().flatten().rev() {
                stack.push((child, depth + 1, Rc::clone(&below)));
            }
        }
    }
    (lists, capped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_path_with_a_parent_is_refused() {
        let root = Path::new("/stick");
        assert_eq!(
            stick_path(root, "/Contents/a.mp3"),
            Some(root.join("Contents").join("a.mp3"))
        );
        assert_eq!(stick_path(root, "/Contents/../x"), None);
    }

    fn node(id: u32, parent: u32, sort_order: u32, name: &str, is_playlist: bool) -> NodeRow {
        NodeRow {
            id,
            parent,
            sort_order,
            name: name.into(),
            is_playlist,
        }
    }

    #[test]
    fn device_playlists_follow_the_tree() {
        // Folders A and B each hold x and y; the rows come in a scrambled order.
        let library = PdbLibrary {
            nodes: vec![
                node(6, 2, 1, "y", true),
                node(3, 1, 0, "x", true),
                node(2, 0, 1, "B", false),
                node(5, 2, 0, "x", true),
                node(4, 1, 1, "y", true),
                node(1, 0, 0, "A", false),
            ],
            entries: vec![(3, 0, 10), (3, 1, 11), (6, 0, 12)],
            ..PdbLibrary::default()
        };
        let ids = HashMap::from([(10, 0), (11, 1), (12, 2)]);
        let (lists, capped) = playlists(&library, &ids);
        assert!(!capped);
        let names: Vec<&str> = lists.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["A / x", "A / y", "B / x", "B / y"]);
        assert_eq!(lists[0].tracks, [0, 1]);
        assert_eq!(lists[3].tracks, [2]);
    }

    #[test]
    fn a_deep_chain_of_long_folder_names_stays_bounded() {
        let mut nodes = Vec::new();
        for id in 1..=31u32 {
            nodes.push(node(id, id - 1, 0, &"x".repeat(4000), false));
        }
        for n in 0..20_000u32 {
            nodes.push(node(100 + n, 31, n, "leaf", true));
        }
        let library = PdbLibrary {
            nodes,
            ..PdbLibrary::default()
        };
        let (lists, capped) = playlists(&library, &HashMap::new());
        assert!(capped);
        assert!(lists.len() <= 10_000);
        assert!(lists.iter().all(|l| l.name.chars().count() <= 1024));
    }
}
