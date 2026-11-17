//! The rekordbox device branch of a guest stick: `PIONEER/rekordbox/export.pdb` and the
//! `PIONEER/USBANLZ` analysis files its tracks point to. `exportLibrary.db` (the newer
//! encrypted library) and `exportExt.pdb` are never read.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::anlz::{AnlzData, read_anlz};
use super::pdb::{NodeRow, PdbLibrary, read_pdb};
use super::{
    GuestApp, GuestPlaylist, GuestSource, GuestTrack, dir_on_stick, on_stick, track_from_tags,
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
    let bytes = match std::fs::metadata(&pdb) {
        Ok(m) if m.len() > MAX_PDB_BYTES => {
            source.problem = Some("export.pdb is too large to read".into());
            return Some(source);
        }
        Ok(_) => std::fs::read(&pdb),
        Err(e) => Err(e),
    };
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) => {
            source.problem = Some(format!("export.pdb could not be read: {e}"));
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
            Ok(Some(data)) => {
                if let Some((first_beat, bpm)) = data.grid {
                    track.first_beat = Some(first_beat);
                    track.bpm = Some(bpm);
                }
                track.hotcues = data.cues;
                track.main_cue = data.main_cue;
            }
            Ok(None) => {}
            Err(Refused::Outside) => counts.anlz_refused += 1,
            Err(Refused::Unreadable) => counts.anlz_unreadable += 1,
        }
        ids.insert(row.id, source.tracks.len());
        source.tracks.push(track);
    }
    source.playlists = playlists(&library, &ids);
    source.problem = counts.problem();
    Some(source)
}

/// What a read had to refuse or could not read.
#[derive(Default)]
struct Counts {
    tracks_refused: usize,
    anlz_refused: usize,
    anlz_unreadable: usize,
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
        if self.anlz_unreadable > 0 {
            out.push(format!(
                "{} analysis file(s) could not be read",
                self.anlz_unreadable
            ));
        }
        (!out.is_empty()).then(|| out.join("; "))
    }
}

enum Refused {
    Outside,
    Unreadable,
}

/// The analysis of a track from its `.DAT` path (and the `.EXT` beside it), if it has one.
fn analysis(root: &Path, analyze_path: &str) -> Result<Option<AnlzData>, Refused> {
    if analyze_path.is_empty() {
        return Ok(None);
    }
    let dat = stick_path(root, analyze_path).ok_or(Refused::Outside)?;
    let dat = on_stick(root, &dat).ok_or(Refused::Outside)?;
    let ext = on_stick(root, &dat.with_extension("EXT"));
    read_anlz(&dat, ext.as_deref())
        .map(Some)
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

/// The playlists of the tree, each named `Folder / Playlist`, with their tracks in order.
fn playlists(library: &PdbLibrary, ids: &HashMap<u32, usize>) -> Vec<GuestPlaylist> {
    let by_id: HashMap<u32, &NodeRow> = library.nodes.iter().map(|n| (n.id, n)).collect();
    let mut lists = Vec::new();
    let mut playlist_nodes: Vec<&NodeRow> =
        library.nodes.iter().filter(|n| n.is_playlist).collect();
    playlist_nodes.sort_by_key(|n| n.sort_order);
    for node in playlist_nodes {
        // The folders above it, outermost first; a cycle in a damaged file stops at its depth.
        let mut names = vec![node.name.as_str()];
        let mut parent = node.parent;
        for _ in 0..64 {
            match by_id.get(&parent) {
                Some(p) if parent != 0 => {
                    names.push(p.name.as_str());
                    parent = p.parent;
                }
                _ => break,
            }
        }
        names.reverse();
        let mut entries: Vec<(u32, u32)> = library
            .entries
            .iter()
            .filter(|(list, _, _)| *list == node.id)
            .map(|&(_, index, track)| (index, track))
            .collect();
        entries.sort_by_key(|&(index, _)| index);
        lists.push(GuestPlaylist {
            name: names.join(" / "),
            tracks: entries
                .into_iter()
                .filter_map(|(_, track)| ids.get(&track).copied())
                .collect(),
        });
    }
    lists
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
}
