//! A reader for rekordbox's `export.pdb` (the DeviceSQL database of a device export), limited
//! to what a guest stick shows: tracks, artist and album names, and the playlist tree.
//!
//! Little-endian. The file is a header and a list of tables; each table is a chain of
//! fixed-size pages, and each page holds rows reached through groups of 16 offsets at its end.
//! Every offset and length is checked against its page before use, so a damaged file gives an
//! error or fewer rows, never a panic or a large allocation. Layout after the public analysis
//! at <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html>.

use std::collections::{HashMap, HashSet};

/// The size of the file header before the table list.
const HEADER_BYTES: usize = 28;
/// One table of the header: type, empty candidate, first page, last page.
const TABLE_BYTES: usize = 16;
/// The size of a page's header; row offsets count from its end.
const PAGE_HEADER_BYTES: usize = 0x28;
/// A row group at the end of a page: 16 offsets, a presence mask and one more word.
const GROUP_BYTES: usize = 36;
/// How many tables the header may list.
const MAX_TABLES: usize = 64;
/// The longest string read; a row declaring more has that string treated as absent.
const MAX_STRING_BYTES: usize = 4096;
/// The smallest and largest page size accepted.
const PAGE_SIZE_RANGE: std::ops::RangeInclusive<usize> = 512..=1 << 20;

const TRACKS: u32 = 0;
const ARTISTS: u32 = 2;
const ALBUMS: u32 = 3;
const PLAYLIST_TREE: u32 = 7;
const PLAYLIST_ENTRIES: u32 = 8;

/// A track row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackRow {
    pub id: u32,
    pub title: String,
    pub artist_id: u32,
    pub album_id: u32,
    /// BPM times 100.
    pub tempo: u32,
    pub file_path: String,
    pub analyze_path: String,
}

/// A node of the playlist tree: a folder or a playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeRow {
    pub id: u32,
    pub parent: u32,
    pub sort_order: u32,
    pub name: String,
    pub is_playlist: bool,
}

/// What the tables of an `export.pdb` hold.
#[derive(Debug, Default)]
pub(crate) struct PdbLibrary {
    pub tracks: Vec<TrackRow>,
    pub artists: HashMap<u32, String>,
    pub albums: HashMap<u32, String>,
    pub nodes: Vec<NodeRow>,
    /// `(playlist id, entry index, track id)`.
    pub entries: Vec<(u32, u32, u32)>,
    /// Whether the row cap stopped the read.
    pub truncated: bool,
    /// The most rows read; set from the file's size.
    pub row_cap: usize,
}

impl PdbLibrary {
    fn row_count(&self) -> usize {
        self.tracks.len()
            + self.artists.len()
            + self.albums.len()
            + self.nodes.len()
            + self.entries.len()
    }
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Reads every table of interest of an `export.pdb`.
pub(crate) fn read_pdb(bytes: &[u8]) -> Result<PdbLibrary, String> {
    let page_size = le32(bytes, 4).ok_or("the file is too short")? as usize;
    if !PAGE_SIZE_RANGE.contains(&page_size) {
        return Err(format!("a page size of {page_size}"));
    }
    let tables = le32(bytes, 8).ok_or("the file is too short")? as usize;
    if tables > MAX_TABLES {
        return Err(format!("{tables} tables"));
    }
    // A chain cannot hold more pages than the file does, and no page is read twice.
    let max_pages = bytes.len() / page_size + 1;
    // A row takes at least 16 bytes of the file, so no more rows than that can be real.
    let mut library = PdbLibrary {
        row_cap: bytes.len() / 16,
        ..PdbLibrary::default()
    };
    let mut visited = HashSet::new();
    let mut kinds_read = HashSet::new();
    for t in 0..tables {
        let at = HEADER_BYTES + t * TABLE_BYTES;
        let (Some(kind), Some(first), Some(last)) =
            (le32(bytes, at), le32(bytes, at + 8), le32(bytes, at + 12))
        else {
            return Err("the table list is cut short".into());
        };
        // Each kind of table is read once, even if the list repeats it.
        if ![TRACKS, ARTISTS, ALBUMS, PLAYLIST_TREE, PLAYLIST_ENTRIES].contains(&kind)
            || !kinds_read.insert(kind)
        {
            continue;
        }
        let mut index = first;
        while visited.len() < max_pages && visited.insert(index) {
            let page = (index as usize)
                .checked_mul(page_size)
                .and_then(|start| bytes.get(start..start.checked_add(page_size)?))
                .ok_or("a page lies beyond the end of the file")?;
            // A page names its own index and table type; a chain that strays is cut.
            if le32(page, 4) != Some(index) || le32(page, 8) != Some(kind) {
                break;
            }
            read_page(page, kind, &mut library);
            if index == last || library.truncated {
                break;
            }
            index = le32(page, 12).ok_or("a page is cut short")?;
        }
    }
    Ok(library)
}

/// Adds the rows of one page to the library.
fn read_page(page: &[u8], kind: u32, library: &mut PdbLibrary) {
    // Pages flagged as holding no data (0x40) are skipped; the row count is the 8-bit field
    // unless the 16-bit one is larger and not 0x1fff.
    let (Some(&flags), Some(&small), Some(large)) =
        (page.get(0x1b), page.get(0x18), le16(page, 0x22))
    else {
        return;
    };
    if flags & 0x40 != 0 {
        return;
    }
    let rows = if large > u16::from(small) && large != 0x1fff {
        usize::from(large)
    } else {
        usize::from(small)
    };
    let mut seen = HashSet::new();
    for group in 0..rows.div_ceil(16) {
        let Some(end) = group
            .checked_add(1)
            .and_then(|g| g.checked_mul(GROUP_BYTES))
            .and_then(|n| page.len().checked_sub(n).map(|start| start + GROUP_BYTES))
            .filter(|&end| end >= PAGE_HEADER_BYTES + GROUP_BYTES)
        else {
            return;
        };
        let Some(present) = le16(page, end - 4) else {
            return;
        };
        for i in 0..16 {
            if present & (1 << i) == 0 {
                continue;
            }
            if library.row_count() >= library.row_cap {
                library.truncated = true;
                return;
            }
            let Some(offset) = le16(page, end - 4 - 2 * (i + 1)) else {
                continue;
            };
            // A row offset used twice on a page is one row.
            if !seen.insert(offset) {
                continue;
            }
            let Some(start) = PAGE_HEADER_BYTES.checked_add(usize::from(offset)) else {
                continue;
            };
            if let Some(row) = page.get(start..) {
                add_row(row, kind, library);
            }
        }
    }
}

fn add_row(row: &[u8], kind: u32, library: &mut PdbLibrary) {
    match kind {
        TRACKS => {
            // The string offsets start at 0x5e: analyze path 14th, title 17th, file path 20th.
            let string = |n: usize| text(row, usize::from(le16(row, 0x5e + 2 * n)?));
            let (Some(tempo), Some(album_id), Some(artist_id), Some(id)) = (
                le32(row, 0x38),
                le32(row, 0x40),
                le32(row, 0x44),
                le32(row, 0x48),
            ) else {
                return;
            };
            library.tracks.push(TrackRow {
                id,
                title: string(17).unwrap_or_default(),
                artist_id,
                album_id,
                tempo,
                file_path: string(20).unwrap_or_default(),
                analyze_path: string(14).unwrap_or_default(),
            });
        }
        ARTISTS => {
            // The name is a byte offset at 9, or a 16-bit one at 10 when the subtype is 0x64.
            let (Some(subtype), Some(id)) = (le16(row, 0), le32(row, 4)) else {
                return;
            };
            let at = if subtype == 0x64 {
                le16(row, 10).map(usize::from)
            } else {
                row.get(9).map(|&b| usize::from(b))
            };
            if let Some(name) = at.and_then(|at| text(row, at)) {
                library.artists.insert(id, name);
            }
        }
        ALBUMS => {
            // Like an artist: a byte offset at 0x15, or a 16-bit one at 0x16 when the subtype
            // is 0x84.
            let (Some(subtype), Some(id)) = (le16(row, 0), le32(row, 12)) else {
                return;
            };
            let at = if subtype == 0x84 {
                le16(row, 0x16).map(usize::from)
            } else {
                row.get(0x15).map(|&b| usize::from(b))
            };
            if let Some(name) = at.and_then(|at| text(row, at)) {
                library.albums.insert(id, name);
            }
        }
        PLAYLIST_TREE => {
            let (Some(parent), Some(sort_order), Some(id), Some(folder)) =
                (le32(row, 0), le32(row, 8), le32(row, 12), le32(row, 16))
            else {
                return;
            };
            library.nodes.push(NodeRow {
                id,
                parent,
                sort_order,
                name: text(row, 0x14).unwrap_or_default(),
                is_playlist: folder == 0,
            });
        }
        PLAYLIST_ENTRIES => {
            if let (Some(index), Some(track), Some(list)) =
                (le32(row, 0), le32(row, 4), le32(row, 8))
            {
                library.entries.push((list, index, track));
            }
        }
        _ => {}
    }
}

/// The DeviceSQL string at `at` in `row`: short ASCII, long ASCII or long UTF-16LE.
fn text(row: &[u8], at: usize) -> Option<String> {
    let flags = *row.get(at)?;
    if flags & 1 == 1 {
        // Short ASCII: the byte holds the length (with itself) shifted left by one.
        let len = usize::from(flags >> 1).checked_sub(1)?;
        let body = row.get(at + 1..at.checked_add(1)?.checked_add(len)?)?;
        return Some(String::from_utf8_lossy(body).into_owned());
    }
    // Long: flags, a 16-bit length counting these four bytes, a pad byte, then the text.
    let len = usize::from(le16(row, at + 1)?).checked_sub(4)?;
    if len > MAX_STRING_BYTES {
        return None;
    }
    let body = row.get(at + 4..at.checked_add(4)?.checked_add(len)?)?;
    match flags {
        0x40 => Some(String::from_utf8_lossy(body).into_owned()),
        0x90 => {
            let units: Vec<u16> = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_in_all_three_forms() {
        // Short ASCII "ab": (2 + 1) << 1 | 1.
        assert_eq!(text(&[0x07, b'a', b'b'], 0).as_deref(), Some("ab"));
        // Long ASCII "hi".
        assert_eq!(text(&[0x40, 6, 0, 0, b'h', b'i'], 0).as_deref(), Some("hi"));
        // Long UTF-16LE "é".
        assert_eq!(
            text(&[0x90, 6, 0, 0, 0xe9, 0], 0).as_deref(),
            Some("\u{e9}")
        );
        // A length running past the row is refused.
        assert_eq!(text(&[0x40, 200, 0, 0, b'h'], 0), None);
        assert_eq!(text(&[], 0), None);
    }

    #[test]
    fn an_oversized_string_is_ignored() {
        // A long ASCII string declaring 5000 bytes, all present, is over the cap.
        let mut row = vec![0x40, 0x8c, 0x13, 0];
        row.extend(vec![b'a'; 5000 - 4]);
        assert_eq!(text(&row, 0), None);
        // Just inside the cap it reads.
        let mut ok = vec![0x40, 0x04, 0x10, 0];
        ok.extend(vec![b'a'; 4096]);
        assert_eq!(text(&ok, 0).map(|t| t.len()), Some(4096));
    }

    #[test]
    fn the_row_cap_follows_the_file_size() {
        let mut library = PdbLibrary {
            row_cap: 2,
            ..PdbLibrary::default()
        };
        let mut page = vec![0u8; 1024];
        page[0x18] = 3;
        let end = 1024 - 36;
        page[end + 32..end + 34].copy_from_slice(&7u16.to_le_bytes());
        for (i, off) in [0x100u16, 0x200, 0x300].iter().enumerate() {
            let at = end + 32 - 2 * (i + 1);
            page[at..at + 2].copy_from_slice(&off.to_le_bytes());
        }
        read_page(&page, PLAYLIST_ENTRIES, &mut library);
        assert!(library.truncated);
        assert_eq!(library.entries.len(), 2);
    }

    #[test]
    fn a_short_or_empty_file_is_an_error() {
        assert!(read_pdb(&[]).is_err());
        assert!(read_pdb(&[0; 64]).is_err());
    }
}
