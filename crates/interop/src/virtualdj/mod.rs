//! VirtualDJ's `database.xml`, playlists and playlist order.

mod database;
mod import;
mod playlists;

use std::path::{Path, PathBuf};

pub(crate) use database::read_database_capped;
pub use database::{VdjPoi, VdjSong, read_database};
pub use import::plan_virtualdj;
pub(crate) use playlists::read_playlists_capped;
pub use playlists::{VdjPlaylist, read_playlists};

/// A failure reading VirtualDJ's files or the library an import goes into.
#[derive(Debug, thiserror::Error)]
pub enum InteropError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not a valid VirtualDJ file: {why}", path.display())]
    Format { path: PathBuf, why: String },
    /// A file larger than a guest-stick read allows; nothing of it was read.
    #[error("{} is too large to read", path.display())]
    TooLarge { path: PathBuf },
    #[error(transparent)]
    Library(#[from] library::LibraryError),
}

/// Where VirtualDJ's data was found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VdjSources {
    pub databases: Vec<PathBuf>,
    pub list_dirs: Vec<PathBuf>,
    pub order: Option<PathBuf>,
}

/// Looks for VirtualDJ's databases and playlists in the home dirs and on the volumes.
pub fn find_sources(home_dirs: &[PathBuf], volumes: &[PathBuf]) -> VdjSources {
    let mut sources = VdjSources::default();
    let volume_dirs: Vec<PathBuf> = volumes.iter().map(|v| v.join("VirtualDJ")).collect();
    for dir in home_dirs.iter().chain(&volume_dirs) {
        let database = dir.join("database.xml");
        if database.is_file() {
            sources.databases.push(database);
        }
        let lists = dir.join("MyLists");
        if lists.is_dir() {
            sources.list_dirs.push(lists);
        }
    }
    sources.order = home_dirs
        .iter()
        .map(|dir| dir.join("MyLists").join("order"))
        .find(|order| order.is_file());
    sources
}

/// The bytes of `path`; with `max`, [`InteropError::TooLarge`] for a larger file, unread.
pub(crate) fn read_file_capped(path: &Path, max: Option<u64>) -> Result<Vec<u8>, InteropError> {
    let io = |source| InteropError::Io {
        path: path.to_path_buf(),
        source,
    };
    let Some(max) = max else {
        return std::fs::read(path).map_err(io);
    };
    crate::guest::read_capped(path, max).map_err(|e| match e {
        crate::guest::ReadError::TooLarge => InteropError::TooLarge {
            path: path.to_path_buf(),
        },
        crate::guest::ReadError::Io(source) => io(source),
    })
}
