//! The writer refuses ops that name a guest track.

use library::{GridEdit, HotcuePoint, Library, LibraryError, RawStatement, WriteOp};
use rusqlite::types::Value;

/// A library with track 1 and one row of every table a guest op could reach.
fn library() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(&dir.path().join("library.sqlite3")).unwrap();
    lib.writer()
        .submit_wait(WriteOp::Raw(vec![RawStatement {
            sql:
                "INSERT INTO tracks(id, path, fingerprint, added_at) VALUES (1, '/m/1.wav', ?1, 0)"
                    .into(),
            params: vec![Value::Blob(vec![0; 32])],
        }]))
        .unwrap();
    (dir, lib)
}

/// Every row count that a refused write could have changed.
fn counts(lib: &Library) -> Vec<i64> {
    let reader = lib.reader().unwrap();
    [
        "tracks",
        "cues",
        "hotcues",
        "analysis",
        "playlists",
        "playlist_tracks",
    ]
    .iter()
    .map(|table| {
        reader
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap_or(-1)
    })
    .collect()
}

#[test]
fn a_write_naming_a_guest_track_is_refused() {
    let (_dir, lib) = library();
    let playlist = match lib
        .writer()
        .submit_wait(WriteOp::CreatePlaylist { name: "p".into() })
        .unwrap()
    {
        library::WriteOutcome::Created { id } => id,
        other => panic!("{other:?}"),
    };
    let before = counts(&lib);
    let ops = [
        WriteOp::SetHotcue {
            track: -1,
            slot: 1,
            frames: 10.0,
            loop_frames: None,
        },
        WriteOp::PutHotcue {
            track: -1,
            slot: 1,
            point: HotcuePoint {
                frames: 10.0,
                loop_frames: None,
                color: None,
                name: None,
            },
        },
        WriteOp::TrackLoaded(-1),
        WriteOp::EditGrid {
            track: -1,
            edit: GridEdit::Halve,
            playhead_frames: None,
            sample_rate: None,
        },
        WriteOp::SetCover {
            track: -1,
            file: "/x.jpg".into(),
            force: true,
        },
        WriteOp::AddToPlaylist {
            playlist,
            track: -1,
            at: None,
        },
        WriteOp::Batch(vec![WriteOp::SetCue {
            track: -1,
            frames: 1.0,
        }]),
    ];
    for op in ops {
        let ids = op.track_ids();
        assert!(ids.contains(&-1), "{op:?} names {ids:?}");
        assert!(matches!(
            lib.writer().submit_wait(op.clone()),
            Err(LibraryError::GuestTrack(-1))
        ));
        assert!(matches!(
            lib.writer().submit(op),
            Err(LibraryError::GuestTrack(-1))
        ));
    }
    assert_eq!(counts(&lib), before);
    assert_eq!(
        LibraryError::GuestTrack(-1).to_string(),
        "track -1 is a guest track; it is never saved to the library"
    );
}

#[test]
fn an_owner_track_is_still_written() {
    let (_dir, lib) = library();
    lib.writer().submit_wait(WriteOp::TrackLoaded(1)).unwrap();
    assert_eq!(WriteOp::TrackLoaded(1).track_ids(), vec![1]);
}
