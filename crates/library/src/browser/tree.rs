//! The browser's folder tree: the drives and the folders opened below them, read from the
//! disk, then the playlists.
//!
//! The tree is held as its visible rows. Opening a folder reads its subfolders and inserts
//! them below it; closing it removes every row below it, so a closed branch is read again
//! when it next opens.

use std::fs;
use std::path::{Path, PathBuf};

use super::Drive;
use super::guest::{GuestKey, GuestList, GuestNode};
use crate::PlaylistSummary;
use crate::scan::is_hidden;

/// What a tree row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    Drive,
    Folder,
    /// The node the playlists sit under.
    Playlists,
    Playlist,
    /// A guest stick, in place of its drive; its apps' branches are below it.
    GuestStick,
    /// One app's part of a guest stick; its lists are below it.
    GuestBranch,
    /// A playlist or crate of a guest stick.
    GuestList,
}

/// One visible row of the folder tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub label: String,
    /// 0 for the drives and the Playlists node.
    pub depth: u8,
    pub kind: TreeKind,
    /// Open, showing its children (the Playlists node always is).
    pub expanded: bool,
    /// Why an open folder shows no children, such as "cannot read".
    pub note: Option<String>,
    /// The library id of a playlist's row; `None` for every other row.
    pub playlist: Option<i64>,
    /// A folder row's folder on disk, as the tree read it; `None` for every other row.
    pub path: Option<PathBuf>,
}

/// What a tree row's list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    /// A drive's or folder's own audio files.
    Dir(PathBuf),
    /// Nothing: the Playlists node.
    Playlists,
    /// A playlist and its entry count.
    Playlist { id: i64, len: usize },
    /// A list of a guest stick: the `list`th of branch `branch` of stick `stick` (as
    /// [`Tree::guests`] has them).
    Guest {
        stick: usize,
        branch: usize,
        list: usize,
    },
    /// Nothing: a guest stick's row (no `branch`) or an app's branch of it.
    GuestHeader { stick: usize, branch: Option<usize> },
}

impl Target {
    /// Whether `self` and `other` are the same entry, even if its contents changed.
    pub(super) fn same_entry(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Playlist { id: a, .. }, Target::Playlist { id: b, .. }) => a == b,
            _ => self == other,
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    row: TreeRow,
    target: Target,
}

/// The visible tree rows. Never empty: the Playlists node is always there.
#[derive(Debug, Clone)]
pub(super) struct Tree {
    nodes: Vec<Node>,
    /// The drives the tree was given, in order.
    drives: Vec<Drive>,
    /// The guest sticks; each shows in place of the drive at its root.
    guests: Vec<GuestNode>,
}

impl Tree {
    /// The drives (closed), then the Playlists node, open, with `playlists` below it.
    pub(super) fn new(roots: Vec<Drive>, playlists: &[PlaylistSummary]) -> Self {
        let mut playlists_node = row("Playlists".to_owned(), 0, TreeKind::Playlists);
        playlists_node.expanded = true;
        let nodes = vec![Node {
            row: playlists_node,
            target: Target::Playlists,
        }];
        let mut tree = Self {
            nodes,
            drives: Vec::new(),
            guests: Vec::new(),
        };
        tree.set_drives(roots);
        tree.set_playlists(playlists);
        tree
    }

    /// Replaces the drives with `roots`, in their order. A drive still there (at the same
    /// path) keeps its row and the folders open below it; a new one comes in closed.
    pub(super) fn set_drives(&mut self, roots: Vec<Drive>) {
        self.drives = roots;
        self.rebuild_drives();
    }

    /// Replaces the guest sticks with `guests`. Each shows in place of the drive at its
    /// root (a stick with no such drive is not shown); the folders open under a stick's
    /// "Files" stay open.
    pub(super) fn set_guests(&mut self, guests: Vec<GuestNode>) {
        self.guests = guests;
        self.rebuild_drives();
    }

    /// The rows of the drives (and guest sticks) again, keeping what is open.
    fn rebuild_drives(&mut self) {
        let at = self
            .nodes
            .iter()
            .position(|n| n.target == Target::Playlists)
            .unwrap_or(self.nodes.len() - 1);
        let old: Vec<Node> = self.nodes.drain(..at).collect();
        let mut rows = Vec::with_capacity(old.len().max(self.drives.len()));
        for drive in &self.drives {
            let stick = self
                .guests
                .iter()
                .position(|g| same_root(&g.root, &drive.path));
            if let Some(stick) = stick {
                guest_rows(&mut rows, &old, drive, stick, &self.guests[stick]);
                continue;
            }
            let target = Target::Dir(drive.path.clone());
            match old.iter().position(|n| {
                n.row.depth == 0 && n.row.kind == TreeKind::Drive && n.target == target
            }) {
                Some(i) => {
                    let start = rows.len();
                    rows.extend(old[i..subtree_end(&old, i)].iter().cloned());
                    rows[start].row.label.clone_from(&drive.name);
                }
                None => rows.push(Node {
                    row: row(drive.name.clone(), 0, TreeKind::Drive),
                    target,
                }),
            }
        }
        self.nodes.splice(0..0, rows);
    }

    /// Guest list `list` of branch `branch` of stick `stick`.
    pub(super) fn guest_list(
        &self,
        stick: usize,
        branch: usize,
        list: usize,
    ) -> Option<&GuestList> {
        self.guests
            .get(stick)?
            .branches
            .get(branch)?
            .lists
            .get(list)
    }

    /// What names the guest row `target` is, if it is one.
    pub(super) fn guest_key(&self, target: &Target) -> Option<GuestKey> {
        let (stick, branch, list) = match *target {
            Target::Guest {
                stick,
                branch,
                list,
            } => (stick, Some(branch), Some(list)),
            Target::GuestHeader { stick, branch } => (stick, branch, None),
            _ => return None,
        };
        let node = self.guests.get(stick)?;
        let branch = branch.and_then(|b| node.branches.get(b));
        Some(GuestKey {
            root: node.root.clone(),
            branch: branch.map(|b| b.label.clone()),
            list: list
                .and_then(|l| branch?.lists.get(l))
                .map(|l| l.name.clone()),
        })
    }

    /// The target of the guest row `key` names, if it is there.
    pub(super) fn guest_target(&self, key: &GuestKey) -> Option<Target> {
        let stick = self.guests.iter().position(|g| g.root == key.root)?;
        let Some(label) = &key.branch else {
            return Some(Target::GuestHeader {
                stick,
                branch: None,
            });
        };
        let branch = self.guests[stick]
            .branches
            .iter()
            .position(|b| b.label == *label)?;
        let Some(name) = &key.list else {
            return Some(Target::GuestHeader {
                stick,
                branch: Some(branch),
            });
        };
        let list = self.guests[stick].branches[branch]
            .lists
            .iter()
            .position(|l| l.name == *name)?;
        Some(Target::Guest {
            stick,
            branch,
            list,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn target(&self, index: usize) -> &Target {
        &self.nodes[index].target
    }

    /// Where `target` is in the tree, if it is visible.
    pub(super) fn position(&self, target: &Target) -> Option<usize> {
        self.nodes.iter().position(|n| n.target.same_entry(target))
    }

    pub(super) fn rows(&self) -> Vec<TreeRow> {
        self.nodes.iter().map(|n| n.row.clone()).collect()
    }

    /// The name of the drive `path` is on: the drive whose folder holds it most closely.
    pub(super) fn drive_of(&self, path: &Path) -> Option<&str> {
        self.nodes
            .iter()
            .filter_map(|n| match (&n.target, n.row.kind) {
                (Target::Dir(dir), TreeKind::Drive) => Some((dir.as_path(), n.row.label.as_str())),
                (
                    Target::GuestHeader {
                        stick,
                        branch: None,
                    },
                    TreeKind::GuestStick,
                ) => Some((
                    self.guests.get(*stick)?.root.as_path(),
                    n.row.label.as_str(),
                )),
                _ => None,
            })
            .filter(|(dir, _)| path.starts_with(dir))
            .max_by_key(|(dir, _)| dir.components().count())
            .map(|(_, label)| label)
    }

    /// Takes out the folders that are no longer on disk (a folder moved to the Trash), with
    /// the rows below them. Drives come and go with [`Tree::set_drives`] instead.
    pub(super) fn drop_gone_folders(&mut self) {
        let mut at = 0;
        while at < self.nodes.len() {
            let node = &self.nodes[at];
            let gone = node.row.kind == TreeKind::Folder
                && matches!(&node.target, Target::Dir(dir) if is_gone(dir));
            if gone {
                let end = self.subtree_end(at);
                self.nodes.drain(at..end);
            } else {
                at += 1;
            }
        }
    }

    /// Row `index`'s label after its ancestors', such as `"Music ▸ House"`.
    pub(super) fn path(&self, index: usize) -> String {
        let mut labels = vec![self.nodes[index].row.label.as_str()];
        let mut depth = self.nodes[index].row.depth;
        for node in self.nodes[..index].iter().rev() {
            if depth == 0 {
                break;
            }
            if node.row.depth < depth {
                depth = node.row.depth;
                labels.push(&node.row.label);
            }
        }
        labels.reverse();
        labels.join(" ▸ ")
    }

    /// Opens or closes the drive or folder at `index`; anything else is left alone.
    ///
    /// Opening reads the folder's subfolders: hidden ones are skipped and the rest sorted
    /// by name, ignoring case. A folder that can't be read opens with no children and the
    /// note "cannot read".
    pub(super) fn toggle(&mut self, index: usize) {
        let Target::Dir(dir) = &self.nodes[index].target else {
            return;
        };
        let dir = dir.clone();
        if self.nodes[index].row.expanded {
            let end = self.subtree_end(index);
            self.nodes.drain(index + 1..end);
            let closed = &mut self.nodes[index].row;
            closed.expanded = false;
            closed.note = None;
            return;
        }
        let read = subfolders(&dir);
        let opened = &mut self.nodes[index].row;
        opened.expanded = true;
        let depth = opened.depth.saturating_add(1);
        match read {
            Ok(children) => {
                let children = children.into_iter().map(|(name, path)| Node {
                    row: TreeRow {
                        path: Some(path.clone()),
                        ..row(name, depth, TreeKind::Folder)
                    },
                    target: Target::Dir(path),
                });
                self.nodes.splice(index + 1..index + 1, children);
            }
            Err(_) => opened.note = Some("cannot read".to_owned()),
        }
    }

    /// Replaces the rows under the Playlists node with `playlists`, in their order.
    pub(super) fn set_playlists(&mut self, playlists: &[PlaylistSummary]) {
        let at = self
            .nodes
            .iter()
            .position(|n| n.target == Target::Playlists)
            .unwrap_or(self.nodes.len() - 1);
        let end = self.subtree_end(at);
        let rows = playlists.iter().map(|p| Node {
            row: TreeRow {
                playlist: Some(p.id),
                ..row(p.name.clone(), 1, TreeKind::Playlist)
            },
            target: Target::Playlist {
                id: p.id,
                len: usize::try_from(p.track_count).unwrap_or(usize::MAX),
            },
        });
        self.nodes.splice(at + 1..end, rows);
    }

    /// The index just past the rows below row `index`.
    pub(super) fn subtree_end(&self, index: usize) -> usize {
        subtree_end(&self.nodes, index)
    }
}

/// The index just past the rows below `nodes[index]`.
fn subtree_end(nodes: &[Node], index: usize) -> usize {
    let depth = nodes[index].row.depth;
    nodes[index + 1..]
        .iter()
        .position(|n| n.row.depth <= depth)
        .map_or(nodes.len(), |i| index + 1 + i)
}

/// Pushes the rows of guest stick `guest` shown in place of `drive`: the stick, each app's
/// branch with its lists, then "Files", the stick's own folders (kept open as they were in
/// `old`).
fn guest_rows(rows: &mut Vec<Node>, old: &[Node], drive: &Drive, stick: usize, guest: &GuestNode) {
    let node = |label: &str, depth, kind, target| Node {
        row: TreeRow {
            expanded: matches!(kind, TreeKind::GuestStick | TreeKind::GuestBranch),
            ..row(label.to_owned(), depth, kind)
        },
        target,
    };
    rows.push(node(
        &drive.name,
        0,
        TreeKind::GuestStick,
        Target::GuestHeader {
            stick,
            branch: None,
        },
    ));
    for (b, branch) in guest.branches.iter().enumerate() {
        let mut branch_node = node(
            &branch.label,
            1,
            TreeKind::GuestBranch,
            Target::GuestHeader {
                stick,
                branch: Some(b),
            },
        );
        branch_node.row.note.clone_from(&branch.note);
        rows.push(branch_node);
        for (l, list) in branch.lists.iter().enumerate() {
            rows.push(node(
                &list.name,
                2,
                TreeKind::GuestList,
                Target::Guest {
                    stick,
                    branch: b,
                    list: l,
                },
            ));
        }
    }
    let target = Target::Dir(drive.path.clone());
    let files = old.iter().position(|n| {
        n.row.depth == 1
            && n.row.kind == TreeKind::Folder
            && n.row.path.is_none()
            && n.target == target
    });
    match files {
        Some(i) => rows.extend(old[i..subtree_end(old, i)].iter().cloned()),
        // Not a folder the owner can delete: it has no path of its own.
        None => rows.push(node("Files", 1, TreeKind::Folder, target)),
    }
}

/// Whether the guest stick mounted at `a` is on the drive at `b`.
fn same_root(a: &Path, b: &Path) -> bool {
    a == b || matches!((dunce::canonicalize(a), dunce::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn row(label: String, depth: u8, kind: TreeKind) -> TreeRow {
    TreeRow {
        label,
        depth,
        kind,
        expanded: false,
        note: None,
        playlist: None,
        path: None,
    }
}

/// True only when `dir` is known not to exist (not when it cannot be checked).
fn is_gone(dir: &Path) -> bool {
    matches!(fs::symlink_metadata(dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// `dir`'s visible subfolders (links to folders included) as `(name, path)`, sorted by
/// name ignoring case.
fn subfolders(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut folders = Vec::new();
    for entry in fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        if is_hidden(&name) {
            continue;
        }
        let path = entry.path();
        let is_dir = match entry.file_type() {
            Ok(t) if t.is_symlink() => path.is_dir(),
            Ok(t) => t.is_dir(),
            Err(_) => false,
        };
        if is_dir {
            folders.push((name.to_string_lossy().into_owned(), path));
        }
    }
    folders.sort_by_cached_key(|(name, _)| name.to_lowercase());
    Ok(folders)
}
