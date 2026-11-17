//! Guest sticks in the browser: sticks prepared by another DJ app, shown read-only. The
//! app hands over plain data (names and the rows of each list, with ids below zero), so
//! nothing here knows how a stick was read; a guest list is searched and sorted in memory,
//! as a playlist is in the database.

use std::cmp::Ordering;
use std::path::PathBuf;

use super::Query;
use crate::{TrackRow, TrackSortKey};

/// A guest stick: its name, where it is mounted and what each app left on it.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestNode {
    pub name: String,
    /// The stick's mount path, as the drive row of the same stick has it.
    pub root: PathBuf,
    pub branches: Vec<GuestBranch>,
}

/// One app's part of a stick.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestBranch {
    /// The app: "Serato", "rekordbox", "rekordbox (xml)" or "VirtualDJ".
    pub label: String,
    /// Why the app's data could not be read in full.
    pub note: Option<String>,
    pub lists: Vec<GuestList>,
}

/// A playlist or crate of a guest stick.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestList {
    pub name: String,
    /// The files of the list in the app's order, each with its row (`id` below zero).
    pub rows: Vec<(PathBuf, TrackRow)>,
}

/// What names a guest tree row across a change of the sticks: the stick's mount path, then
/// the branch's label and the list's name, as far as the row goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuestKey {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub list: Option<String>,
}

/// `list`'s rows that match `query`, in the sort's order or the list's own.
pub(super) fn rows_for(list: &GuestList, query: &Query) -> Vec<(PathBuf, TrackRow)> {
    let mut rows: Vec<(PathBuf, TrackRow)> = match &query.text {
        None => list.rows.clone(),
        Some(text) => {
            // SQLite's LIKE ignores ASCII case only; so does this.
            let needle = text.to_ascii_lowercase();
            let has = |s: &Option<String>| {
                s.as_deref()
                    .is_some_and(|s| s.to_ascii_lowercase().contains(&needle))
            };
            list.rows
                .iter()
                .filter(|(path, row)| {
                    has(&row.title)
                        || has(&row.artist)
                        || row.title.is_none()
                            && path.file_name().is_some_and(|n| {
                                n.to_string_lossy().to_ascii_lowercase().contains(&needle)
                            })
                })
                .cloned()
                .collect()
        }
    };
    if let Some(sort) = query.sort {
        // Stable: equal values keep the list's order. Unknown values come first ascending,
        // last descending, as the library's track queries sort.
        rows.sort_by(|(_, a), (_, b)| {
            let order = compare(sort.key, a, b);
            if sort.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
    rows
}

fn compare(key: TrackSortKey, a: &TrackRow, b: &TrackRow) -> Ordering {
    let text = |s: &Option<String>| s.as_ref().map(|s| s.to_lowercase());
    match key {
        TrackSortKey::Title => text(&a.title).cmp(&text(&b.title)),
        TrackSortKey::Artist => text(&a.artist).cmp(&text(&b.artist)),
        TrackSortKey::Bpm => cmp_float(a.bpm, b.bpm),
        TrackSortKey::Key => a.key.cmp(&b.key),
        TrackSortKey::Length => cmp_float(seconds(a), seconds(b)),
        // A stick holds no date the app could name.
        TrackSortKey::Added => Ordering::Equal,
    }
}

fn cmp_float(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => a.total_cmp(&b),
    }
}

#[allow(clippy::cast_precision_loss)]
fn seconds(row: &TrackRow) -> Option<f64> {
    let rate = row.sample_rate.filter(|&r| r > 0)?;
    Some(row.duration_frames? as f64 / f64::from(rate))
}

/// The total length of `rows` in seconds (those with a length).
pub(super) fn total_seconds(rows: &[(PathBuf, TrackRow)]) -> f64 {
    rows.iter().filter_map(|(_, row)| seconds(row)).sum()
}
