//! The computer's drives: the roots of the browser's folder tree.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
use crate::scan::is_hidden;

/// A drive at the top of the folder tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// What the tree shows, such as "Macintosh HD".
    pub name: String,
    pub path: PathBuf,
}

/// Which apps' folders a volume's root holds, so it is a stick prepared by someone else's
/// DJ software (a guest stick) and not one of the owner's own folders. Only the root is
/// looked at, and nothing is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuestMarkers {
    /// A `_Serato_` folder.
    pub serato: bool,
    /// A `PIONEER/rekordbox/export.pdb` file (a rekordbox device export).
    pub rekordbox_device: bool,
    /// A `rekordbox.xml` file.
    pub rekordbox_xml: bool,
    /// A `VirtualDJ` folder.
    pub virtualdj: bool,
}

impl GuestMarkers {
    /// Whether any app's marker is there.
    #[must_use]
    pub fn any(&self) -> bool {
        self.serato || self.rekordbox_device || self.rekordbox_xml || self.virtualdj
    }
}

/// The markers on `root`'s own level: a `_Serato_` folder, `PIONEER/rekordbox/export.pdb`,
/// `rekordbox.xml` and a `VirtualDJ` folder, and nothing deeper.
#[must_use]
pub fn guest_markers(root: &Path) -> GuestMarkers {
    GuestMarkers {
        serato: root.join("_Serato_").is_dir(),
        rekordbox_device: root
            .join("PIONEER")
            .join("rekordbox")
            .join("export.pdb")
            .is_file(),
        rekordbox_xml: root.join("rekordbox.xml").is_file(),
        virtualdj: root.join("VirtualDJ").is_dir(),
    }
}

/// The computer's drives, the system drive first.
///
/// - macOS: `/`, named after the `/Volumes` link to it ("Macintosh HD" if there is none),
///   then every other entry of `/Volumes`.
/// - Windows: every drive letter that exists.
/// - Linux and others: `/` ("Computer"), then the entries of `/media/$USER` and `/mnt`.
///
/// Hidden (`.`-prefixed) entries are skipped. Nothing below a drive is read.
#[must_use]
pub fn drives() -> Vec<Drive> {
    #[cfg(target_os = "macos")]
    {
        mac_drives(Path::new("/Volumes"))
    }
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(char::from)
            .filter_map(|letter| {
                let path = PathBuf::from(format!("{letter}:\\"));
                path.exists().then(|| Drive {
                    name: format!("{letter}:"),
                    path,
                })
            })
            .collect()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mut drives = vec![Drive {
            name: "Computer".to_owned(),
            path: PathBuf::from("/"),
        }];
        if let Some(user) = std::env::var_os("USER") {
            drives.extend(entries(&Path::new("/media").join(user)));
        }
        drives.extend(entries(Path::new("/mnt")));
        drives
    }
}

/// `/` named after its link in `volumes`, then `volumes`' other entries.
#[cfg(target_os = "macos")]
fn mac_drives(volumes: &Path) -> Vec<Drive> {
    let mut root_name = None;
    let mut others = Vec::new();
    for drive in entries(volumes) {
        // Only the link itself is read, never what is inside the volume.
        match fs::read_link(&drive.path) {
            Ok(target) if target == Path::new("/") => {
                root_name.get_or_insert(drive.name);
            }
            _ => others.push(drive),
        }
    }
    let root = Drive {
        name: root_name.unwrap_or_else(|| "Macintosh HD".to_owned()),
        path: PathBuf::from("/"),
    };
    std::iter::once(root).chain(others).collect()
}

/// The visible entries of `dir` as drives, sorted by name (none if it can't be read).
#[cfg(not(windows))]
fn entries(dir: &Path) -> Vec<Drive> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut drives: Vec<Drive> = read
        .flatten()
        .filter(|e| !is_hidden(&e.file_name()))
        .map(|e| Drive {
            name: e.file_name().to_string_lossy().into_owned(),
            path: e.path(),
        })
        .collect();
    drives.sort_by_cached_key(|d| d.name.to_lowercase());
    drives
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_link_to_root_names_the_system_drive() {
        let volumes = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/", volumes.path().join("System Disk")).unwrap();
        fs::create_dir(volumes.path().join("Music")).unwrap();
        fs::create_dir(volumes.path().join(".timemachine")).unwrap();
        std::os::unix::fs::symlink("/tmp", volumes.path().join("Elsewhere")).unwrap();
        let names: Vec<_> = mac_drives(volumes.path())
            .into_iter()
            .map(|d| (d.name, d.path))
            .collect();
        assert_eq!(
            names,
            [
                ("System Disk".to_owned(), PathBuf::from("/")),
                ("Elsewhere".to_owned(), volumes.path().join("Elsewhere")),
                ("Music".to_owned(), volumes.path().join("Music")),
            ]
        );

        // No link to `/`: the default name.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(mac_drives(empty.path())[0].name, "Macintosh HD");
    }
}

#[cfg(test)]
mod guest_tests {
    use std::fs;

    use super::*;

    #[test]
    fn guest_markers_finds_each_app() {
        let root = tempfile::tempdir().unwrap();
        let at = |name: &str| root.path().join(name);
        assert!(!guest_markers(root.path()).any());

        fs::create_dir(at("_Serato_")).unwrap();
        let found = guest_markers(root.path());
        assert!(found.serato && found.any());
        assert!(!found.rekordbox_device && !found.rekordbox_xml && !found.virtualdj);

        fs::create_dir_all(at("PIONEER/rekordbox")).unwrap();
        assert!(!guest_markers(root.path()).rekordbox_device);
        fs::write(at("PIONEER/rekordbox/export.pdb"), b"").unwrap();
        assert!(guest_markers(root.path()).rekordbox_device);

        // A folder named like the xml file is not the file.
        fs::create_dir(at("rekordbox.xml")).unwrap();
        assert!(!guest_markers(root.path()).rekordbox_xml);
        fs::remove_dir(at("rekordbox.xml")).unwrap();
        fs::write(at("rekordbox.xml"), b"").unwrap();
        assert!(guest_markers(root.path()).rekordbox_xml);

        // A file named like the VirtualDJ folder is not the folder.
        fs::write(at("VirtualDJ"), b"").unwrap();
        assert!(!guest_markers(root.path()).virtualdj);
        fs::remove_file(at("VirtualDJ")).unwrap();
        fs::create_dir(at("VirtualDJ")).unwrap();
        assert!(guest_markers(root.path()).virtualdj);

        // Only the root counts, not what is deeper.
        let deep = tempfile::tempdir().unwrap();
        fs::create_dir_all(deep.path().join("music/_Serato_")).unwrap();
        assert!(!guest_markers(deep.path()).any());
    }
}
