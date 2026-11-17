# 11b: Guest Sticks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A USB stick made by Serato, rekordbox (device library or `rekordbox.xml`) or VirtualDJ shows in the browser
while it is plugged in, with its playlists in order. Its tracks load on the decks with their hotcues, saved loops, main
cue and grid. Nothing of it ever reaches the owner's library, and nothing is ever written to the stick.

**Architecture:**
- **`interop::guest` reads a stick** into a plain model: one `GuestSource` per app found, each with tracks (positions in
  seconds) and playlists (indexes into its tracks).
- **The app owns a `GuestSticks` store.** It assigns every guest track a session id below zero; SQLite ids are always
  positive, so the two never collide.
- **The browser shows the store as plain `library::browser` data.** The library crate never depends on `interop`.
- **`LibraryMode` routes negative ids** to the store instead of the owner's `Reader`/`Writer`.
- **`Writer` itself refuses any op that names a negative track id.** This is the backstop for every write path.

**Tech Stack:** Rust 2024, quick-xml (already used), lofty (already used), `rekordcrate` 0.3.0 for `export.pdb` only
(MPL-2.0, allowed by `deny.toml`), our own parser for rekordbox `ANLZ` tags (rekordcrate 0.3.0 fails on rekordbox 7's
cue lists: "Unexpected value for enum: 1" in `CueList.cue_type`).

**Spec:** `docs/superpowers/specs/2026-11-09-interop-design.md` — sections "Guest sticks", Goals 2, Anti-goals,
"Positions, grids and colours", Verification.

**Fixture:** `crates/interop/tests/fixtures/guest/` (committed with this plan). A real stick made by the owner with
Serato DJ Lite, rekordbox 7.2.19 and VirtualDJ 2026. Its `README.md` lists every expected value used below.

## Global Constraints

- **Never write to a guest's stick.** Reading only, not even a cache or an index file.
- **Nothing of a guest stick is added to the owner's library:** no track, cue, grid, cover, stems row or playlist
  entry. Guest hotcue edits live in memory for the session only.
- The stick appears while plugged in and disappears when removed. A deck playing a guest track keeps playing what
  is decoded and reports the stick as gone.
- Only volumes carrying an app marker count (`_Serato_/`, `PIONEER/rekordbox/export.pdb`, a root `rekordbox.xml`,
  `VirtualDJ/`). A volume holding any of the owner's library folders is never a guest stick.
- Several apps on one stick each show as their own branch under the stick node.
- Nested playlist folders are flattened to "Folder / Playlist".
- A guest track uses its app's grid when the stick has one.
- Stems on guest tracks are out (non-goal). So are CDJ playback claims and the encrypted `exportLibrary.db`.
- Tests never touch /Volumes, a real stick, the network, the owner's data or VirtualDJ folders. They use the fixture
  or temp dirs, with Windows-safe paths (one `join()` per part; `dunce` to canonicalise).
- Rust, `cargo deny`, macOS/Windows/Linux builds, Conventional Commits, `Co-Authored-By` trailer.
- The control thread never reads a stick or blocks on one. Mount checks and stick reading run off it.

## Review Focus

1. **A stick pulled out mid-set.** A deck playing a guest track keeps playing and says the stick is gone. Loading
   from a removed stick fails with a clear message. Its browser node goes within a few seconds. (Task 6, Task 8 tests.)
2. **The owner's music drive carries a `VirtualDJ/` folder** (VirtualDJ's per-drive database). It must never become a
   guest stick. Browsing any folder of a guest stick must never add files to the owner's library. (Task 1, Task 6.)
3. **Cue or loop positions past the end of the audio, or negative.** These are dropped, never a crash. The committed
   fixture audio is cut short, so this is exercised for real. (Task 8.)
4. **One app's data on the stick is broken or half-written.** The other apps' branches still show; the broken one
   shows a note and a message names the file. (Task 2.)
5. **The same file listed by two apps, or two sticks with the same volume name.** Each gets distinct ids and its own
   node; edits to one's hotcues don't show on the other. (Task 6.)

---

### Task 1: The owner's library refuses guest ids, and guest sticks are never added

**Files:**
- Modify: `crates/library/src/writer.rs` (`Writer::submit`, `submit_wait`; a `WriteOp::track_ids(&self) -> Vec<i64>`)
- Modify: `crates/library/src/browser/drives.rs` (`guest_markers(root: &Path) -> GuestMarkers`)
- Modify: `crates/app/src/library_mode/worker.rs` (`FolderAdder`, ~476-537)
- Test: `crates/library/tests/writer.rs` (or the writer's test module), `crates/app/src/library_mode/worker.rs` tests

**Interfaces:**
- Produces:
  - `library::WriteOp::track_ids(&self) -> Vec<i64>`: every track id the op names.
  - `Writer::submit`/`submit_wait` return `Err(LibraryError::GuestTrack(id))` for any negative id, and nothing is
    queued. Add the variant, with the message "track {id} is a guest track; it is never saved to the library".
  - `library::browser::GuestMarkers { serato: bool, rekordbox_device: bool, rekordbox_xml: bool, virtualdj: bool }`
    with `any(&self) -> bool`.
  - `guest_markers(root)` checks only these paths, and nothing deeper:
    - `_Serato_` dir;
    - `PIONEER/rekordbox/export.pdb` file;
    - `rekordbox.xml` file;
    - `VirtualDJ` dir.
  - `FolderAdder` skips any folder whose volume has `guest_markers(..).any()` and holds no library folder. The volume
    comes from `volume_of` in `crates/app/src/sticks.rs` and the library folders from `library_volumes`.

- [ ] **Step 1: Write the failing tests.**
  - `a_write_naming_a_guest_track_is_refused`: `SetHotcue`, `PutHotcue`, `TrackLoaded`, `EditGrid`, `SetCover` and
    `AddToPlaylist` with id `-1` each return `GuestTrack(-1)`, and the database is unchanged.
  - `guest_markers_finds_each_app`: built on a temp dir.
  - `browsing_a_guest_stick_adds_nothing_to_the_library`: a temp "volume" with `_Serato_/` and a WAV. Highlighting
    its folder leaves `tracks` empty.
  - `a_drive_holding_a_library_folder_is_still_added`: the owner's drive with `VirtualDJ/` keeps today's behaviour.
- [ ] **Step 2:** Run `cargo test -p library -p open-jockey` and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again; they pass.
- [ ] **Step 5:** Commit: `fix(library): guest tracks never reach the library, and guest sticks are never added`

### Task 2: The guest model and the VirtualDJ branch

**Files:**
- Create: `crates/interop/src/guest/mod.rs` (the model and `read_stick`)
- Create: `crates/interop/src/guest/virtualdj.rs`
- Modify: `crates/interop/src/lib.rs` (`pub mod guest;`)
- Modify: `crates/interop/src/virtualdj/database.rs` (`VdjSong` gains `title`, `artist` from `<Tags>`)
- Test: `crates/interop/tests/guest_virtualdj.rs`

**Interfaces:**
- Produces, in `interop::guest`:

```rust
pub enum GuestApp { Serato, RekordboxDevice, RekordboxXml, VirtualDj }
pub struct GuestStick { pub root: PathBuf, pub name: String, pub sources: Vec<GuestSource> }
pub struct GuestSource {
    pub app: GuestApp,
    pub tracks: Vec<GuestTrack>,
    pub playlists: Vec<GuestPlaylist>,
    /// Why this app's data could not be read in full; shown on its branch.
    pub problem: Option<String>,
}
pub struct GuestPlaylist { pub name: String, pub tracks: Vec<usize> }   // indexes into `tracks`, in order
pub struct GuestTrack {
    pub path: PathBuf,                       // on the stick, absolute
    pub title: Option<String>, pub artist: Option<String>, pub album: Option<String>,
    pub bpm: Option<f64>, pub first_beat: Option<f64>,   // seconds
    pub main_cue: Option<f64>,               // seconds
    pub hotcues: Vec<GuestCue>,
}
pub struct GuestCue { pub slot: u8 /* 0..8 */, pub seconds: f64, pub loop_seconds: Option<f64>,
                      pub color: Option<u32> /* 0xRRGGBB */, pub name: Option<String> }
pub fn read_stick(root: &Path, name: &str) -> GuestStick
```

- How `read_stick` works:
  - It reads every app whose marker is present. A failure in one app becomes that source's `problem` and never stops
    the others.
  - A playlist entry pointing at a file missing on the stick is dropped. The source's `problem` says how many were
    dropped.
  - A track listed by a playlist but not by the app's database still gets a `GuestTrack` with no cues.
- VirtualDJ:
  - Call `read_database(<root>/VirtualDJ/database.xml, Some(root))`, which maps `E:/…` to `root`.
  - For each `.vdjfolder` in `<root>/VirtualDJ/MyLists`, map its paths onto the stick. A path whose second component,
    after a mount dir such as `/Volumes/<name>/`, or after a drive letter `X:/`, matches is rejoined onto `root`.
  - Loops: `VdjPoi::Loop{beats}` becomes `loop_seconds = beats × seconds_per_beat`. With no grid, the loop is dropped.
  - Slots: VirtualDJ `num` 1..8 becomes slot 0..7. A higher num is dropped.

- [ ] **Step 1: Write the failing tests** against the fixture.
  - `virtualdj_branch_reads_the_list_in_order`: `OJ VDJ A` gives the titles "OJ Click 126", "OJ Click 120",
    "OJ Click 128".
  - `virtualdj_cues_names_and_loop`:
    - 126 has slots 0, 1 and 2 at 10.554195, 28.60 and 46.072268 s, named "Test 1", "TEst 2" and "Test 3";
    - slot 3 is a loop at 38.120 s, 8 beats long (60/126 × 8 s ≈ 3.8095);
    - colours are `None`.
  - `a_broken_database_keeps_the_other_apps`:
    - copy the fixture to a temp dir and truncate `VirtualDJ/database.xml`;
    - the VirtualDJ source has a `problem` and the Serato source still has 3 tracks.
  - `paths_on_another_mount_name_map_to_the_stick`: the `/Volumes/OJGUEST/…` entries resolve under the temp copy's
    root.
- [ ] **Step 2:** Run `cargo test -p interop --test guest_virtualdj` and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** The tests pass.
- [ ] **Step 5:** Commit: `feat(interop): read a guest stick's VirtualDJ lists and cues`

### Task 3: The Serato branch

**Files:**
- Create: `crates/interop/src/guest/serato.rs`
- Modify: `crates/interop/src/export/serato/{tlv.rs,tags.rs,timing.rs}`, to make the readers `pub(crate)`:
  `tlv::decode`, `utf16be`, `tags::read_id3`, `serato_frames`
- Test: `crates/interop/tests/guest_serato.rs`

**Interfaces:**
- Consumes: the Task 2 model.
- What it reads:
  - Tracks: `_Serato_/database V2`, TLV `otrk` records with `pfil` (the path relative to the volume root), `tsng`,
    `tart`, `talb` and `tbpm`.
  - Playlists: `_Serato_/Subcrates/*.crate`, with the `ptrk` paths in order.
    - The crate name is the file stem.
    - Serato's `%%` separator in a stem becomes " / ".
    - Order: by `_Serato_/neworder.pref` when it lists them, else by name.
  - Cues: from each file's `Serato Markers2` GEOB (MP3 ID3, or the WAV `id3 ` chunk).
    - `CUE`: the index becomes the slot; the position is in ms; the colour is RGB; the name is kept if non-empty.
    - `LOOP` becomes `loop_seconds`.
    - Other entries are ignored.
  - MP3 offset: subtract the inverse of `SERATO_INFO_FRAME_FRAMES` (timing.rs) for an MP3 with an Info frame, exactly
    as export adds it.

- [ ] **Step 1: Write the failing tests.**
  - `serato_crates_in_order`: "OJ Serato A" is 128, 120, 126 and "OJ Serato B" is 126.
  - `serato_cues_from_the_files`: the 120.wav slots 0..3 at 10.944, 26.161, 44.277 and 35.786 s, coloured 0xCC0000,
    0xCC8800, 0x0000CC and 0xCCCC00.
  - `serato_mp3_offset_round_trips_with_export`: a Markers2 written by `export::serato::markers2_geob` for an MP3
    with an Info frame reads back at the original seconds, within 1 ms.
  - `a_file_with_no_markers_has_no_cues`.
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass.
- [ ] **Step 5:** Commit: `feat(interop): read a guest stick's Serato crates and cues`

### Task 4: The rekordbox XML branch

**Files:**
- Create: `crates/interop/src/guest/rekordbox_xml.rs`
- Test: `crates/interop/tests/guest_rekordbox_xml.rs`

**Interfaces:**
- Track fields:
  - `<TRACK>` gives `Name`, `Artist`, `Album` and `Location`;
  - the first `<TEMPO>` gives `Bpm` and `Inizio`;
  - `<POSITION_MARK>` gives the marks.
- `Location`:
  - It is percent-decoded from `file://localhost/…`.
  - If it lies outside the stick, it is matched by the path after the stick's name, else by file name under the
    stick root.
  - With no match, the track is dropped and counted in `problem`.
- `POSITION_MARK`:
  - `Num` 0..7 and `Type` 0 make a hotcue.
  - `Type` 4 makes a loop, with `loop_seconds = End − Start`.
  - `Num` −1 makes a memory cue; the first one becomes `main_cue`.
  - Colour comes from `Red`/`Green`/`Blue`, and `Name` is kept if non-empty.
- Playlists: `<NODE Type="1">` nodes in document order. Folders (`Type="0"`, other than ROOT) are flattened to
  "Folder / Name".

- [ ] **Step 1: Write the failing tests.**
  - `rekordbox_xml_playlist_in_order`: "OJ Rekordbox A" is 120, 128, 126.
  - `rekordbox_xml_marks`:
    - 120 has slot 0 at 5.216 s, "Test 3", 0xB4BE04;
    - slot 3 is a loop at 30.422 s, 1.601 s long, 0xFF8C00;
    - 128 has `main_cue == Some(0.0)`.
  - `rekordbox_xml_uses_its_grid_even_when_wrong`: 120's bpm is 149.96 and first_beat is 0.015.
  - `nested_folders_are_flattened`: an inline XML string with a folder "Sets" holding "Friday" gives
    "Sets / Friday".
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass.
- [ ] **Step 5:** Commit: `feat(interop): read a guest stick's rekordbox.xml`

### Task 5: The rekordbox device branch

**Files:**
- Modify: the workspace `Cargo.toml` and `crates/interop/Cargo.toml`: add `rekordcrate = { version = "0.3.0",
  default-features = false }` and `binrw = "0.14"` (the version rekordcrate uses). Then run `cargo deny check`.
- Create: `crates/interop/src/guest/rekordbox_device.rs` (pdb through rekordcrate)
- Create: `crates/interop/src/guest/anlz.rs` (our own `ANLZ` parser)
- Test: `crates/interop/tests/guest_rekordbox_device.rs`

**Interfaces:**
- pdb, read with `rekordcrate::pdb::{Header, Row}` using `Header::read_be`:
  - tracks: `Row::Track`, with the file path, title, artist/album via their tables, `tempo/100` and `analyze_path`;
  - the playlist tree: `Row::PlaylistTreeNode`;
  - entries: `Row::PlaylistEntry`, sorted by `entry_index`.
  - Folders are flattened as in Task 4.
- `anlz` (our own) reads the `PMAI` header, then these tags, big-endian, from `.DAT` then `.EXT`:
  - `PQTZ`: the beat grid. The first beat's time gives `first_beat`, and its tempo/100 gives `bpm`.
  - `PCOB` (`.DAT`) and `PCO2` (`.EXT`, preferred when present): cue lists.
    - Hot cues 1..8 become slots 0..7.
    - Memory cues: the first one becomes `main_cue`.
    - Loops: `loop_seconds`.
    - `PCO2` adds colour (RGB) and a UTF-16 comment, which becomes the name.
  - `pub(crate) fn read_anlz(dat: &Path, ext: Option<&Path>) -> Result<AnlzData, InteropError>`
  - `pub(crate) struct AnlzData { pub grid: Option<(f64, f64)>, pub cues: Vec<GuestCue>, pub main_cue: Option<f64> }`
  - Any tag it doesn't know is skipped by its length, never an error.
- `PIONEER/rekordbox/exportLibrary.db` is never read.

- [ ] **Step 1: Write the failing tests.**
  - `device_playlist_in_order`: "OJ Rekordbox A" is 120, 128, 126, and the paths are under `Contents/`.
  - `device_grid_from_anlz`: 120 is 149.96 BPM, and 126 is 126.00 BPM.
  - `device_cues_match_the_xml`: every hotcue, loop and main cue of the three tracks equals Task 4's values within
    1 ms, with the same colours and names.
  - `an_anlz_with_an_unknown_tag_still_reads`: an in-memory ANLZ with an extra `PXYZ` tag gives the same cues.
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass, and `cargo deny check` is clean.
- [ ] **Step 5:** Commit: `feat(interop): read a guest stick's rekordbox device library`

### Task 6: Finding sticks, and the guest store

**Files:**
- Create: `crates/app/src/guest.rs` (`GuestSticks`)
- Modify: `crates/app/src/sticks.rs` (reuse `Mounts`, `library_volumes`)
- Modify: `crates/app/src/library_mode/{mod.rs,worker.rs}` (a mount check on the library-worker every 2 s; stick
  reading on a "guest-reader" thread)
- Test: in `crates/app/src/guest.rs`, plus `library_mode` tests

**Interfaces:**
- Produces:
  - `GuestSticks` is held behind an `Arc` and shared with the library-worker, locked only for short copies.
  - Methods:
    - `sticks(&self) -> Vec<StickSummary>`;
    - `track(&self, id: i64) -> Option<GuestTrackRef>`, carrying the row, path, `GuestTrack` and the stick's root;
    - `set_hotcues(&self, id: i64, memory: engine::Memory)`, kept for the session;
    - `is_guest(id) -> bool`, which is `id < 0`.
  - Ids: a counter from −1 downwards, never reused in a session. Each (stick mount, app, track index) gets its own id,
    so the same file in two apps has two ids.
  - Plugging in again gives new ids. A session edit dies with the stick.
- How sticks are found:
  - Every 2 s the worker lists `Mounts::usual()`'s dirs (the drive letters on Windows).
  - For a new entry, `guest_markers(root).any()` must hold and the entry must not be one of `library_volumes`.
  - Then `interop::guest::read_stick` runs on the guest-reader thread, and its result comes back as an event.
  - A vanished entry is removed and the event says so.
  - The control thread only drains events.
- Messages:
  - "Stick {name}: Serato, rekordbox" when found.
  - "Stick {name} removed" when removed.
  - "Stick {name}: {problem}" for each source problem.

- [ ] **Step 1: Write the failing tests.**
  - `a_marked_volume_becomes_a_guest_stick`: a temp mount dir holding a copy of the fixture as `OJGUEST`.
  - `the_owners_music_drive_is_never_a_guest`: a volume holding a library folder plus `VirtualDJ/`.
  - `removing_the_stick_removes_it`.
  - `two_sticks_with_one_name_stay_apart`: two mount dirs.
  - `the_same_file_in_two_apps_has_two_ids`.
  - `guest_ids_are_negative_and_never_reused`.
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass.
- [ ] **Step 5:** Commit: `feat(app): find guest sticks as they are plugged in and removed`

### Task 7: The stick in the browser

**Files:**
- Modify: `crates/library/src/browser/{mod.rs,tree.rs}`
- Modify: `crates/library/src/lib.rs` (re-exports)
- Modify: `crates/ui/src/browser.rs` (the new `TreeKind`s)
- Modify: `crates/app/src/library_mode/worker.rs` (pass the stick data to the browser)
- Test: `crates/library/tests/browser.rs`, `crates/ui/tests/browser.rs`

**Interfaces:**
- Produces (library, plain data, no `interop` dependency):

```rust
pub struct GuestNode { pub name: String, pub root: PathBuf, pub branches: Vec<GuestBranch> }
pub struct GuestBranch { pub label: String /* "Serato", "rekordbox", "rekordbox (xml)", "VirtualDJ" */,
                         pub note: Option<String>, pub lists: Vec<GuestList> }
pub struct GuestList { pub name: String, pub rows: Vec<(PathBuf, TrackRow)> }  // TrackRow.id < 0
impl Browser { pub fn set_guests(&mut self, guests: Vec<GuestNode>) }
```

- New tree kinds:
  - `TreeKind::GuestStick`, `GuestBranch` and `GuestList`.
  - `Target::Guest { stick: usize, branch: usize, list: usize }`.
- Where the stick shows:
  - A guest stick shows **in place of** its plain `Drive` row, with the same name and its branches below. Its folders
    are still browsable under a "Files" child. Those files can't be loaded (they have no library track), and Task 1
    stops them being added. (Ruling R17, final review: so every track still loads, each app branch gets an
    "All tracks" list first, built from the source's tracks.)
- How guest lists behave:
  - A guest list uses search and sort like a playlist.
  - `BrowserView.playlist` stays `None` for a guest list, so playlist edits never target it.
- Window and keyboard:
  - Drag from a guest list onto a deck works.
  - Dropping guest rows onto a playlist is refused with "Guest tracks can't be added to your playlists".
  - Cmd+A works; Delete on a guest list does nothing and shows the message "A guest stick is never changed".
- The footer's left slot reads "{list} · N tracks · length", like a playlist.

- [ ] **Step 1: Write the failing tests.**
  - `a_guest_stick_shows_its_branches_and_lists`: rows for "OJGUEST", then "Serato", "OJ Serato A" and so on.
  - `a_guest_list_keeps_its_order_and_searches`.
  - `dropping_a_guest_row_on_a_playlist_is_refused`.
  - `removing_the_stick_moves_the_highlight_off_it`.
  - `delete_on_a_guest_list_changes_nothing`.
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass.
- [ ] **Step 5:** Commit: `feat(library): guest sticks and their playlists in the browser`

### Task 8: Loading and playing guest tracks

**Files:**
- Modify: `crates/app/src/library_mode/{mod.rs,worker.rs}` (`load_track`, `request_load`, `read_memory`,
  `save_memory`, `request_stems`, `Job::Refresh`, `Job::Grid`, hotcue labels, `CoverFiles`)
- Modify: `crates/app/src/session/{commands.rs,detail.rs}` (cover commands and the detail waveform for guest ids)
- Test: `library_mode` and session tests

**Interfaces:**
- Consumes: `GuestSticks::track`, `set_hotcues` (Task 6), and the browser rows (Task 7).
- Loading:
  - `Load{deck, track}` with `track < 0` is decoded from the stick path.
  - The engine `Memory` is built from the `GuestTrack`'s seconds at the decoded rate:
    `frames = seconds × sample_rate`.
  - A position `< 0`, or at or past the decoded length, is dropped, and so is a loop that runs past the end. The
    message gives the count: "Deck N: 2 cues of {title} are past the end of the file".
  - The grid comes from `bpm` and `first_beat`. (Ruling R18, final review: only when both are there and the BPM is
    within `MIN_GRID_BPM..=MAX_GRID_BPM`; otherwise the track counts as gridless and gets in-memory detection.)
  - With no grid, the loader runs `analysis` BPM detection on the decoded buffer, off the control thread, and sets
    the grid only in memory.
- Saving:
  - Hotcue edits, labels and grid edits on a guest deck go to `GuestSticks::set_hotcues` and memory, never to the
    `Writer`.
- Skipped for guest ids: `TrackLoaded`, stems requests, cover lookups and edits (the "Find another cover" and
  "Remove cover" items are disabled), and `Job::Refresh` reads.
- The detail waveform:
  - It takes hotcue colours and names from the deck's memory, not from `reader.hotcue_points`.
  - It computes the overview from the decoded buffer, because there is no stored overview.
- When the stick is removed:
  - A playing guest deck keeps playing and its title shows "(stick removed)".
  - Loading a gone guest id fails with "Stick {name} is no longer plugged in".

- [ ] **Step 1: Write the failing tests.** Use the fixture copied into a temp mount.
  - `a_guest_track_loads_with_its_cues`: VirtualDJ 126 loads; its hotcues are at 10.554195 s × rate, slot 3 is an
    8-beat loop; and the library's `tracks`, `cues`, `hotcues` and `analysis` tables are empty.
  - `cues_past_the_end_are_dropped_with_a_message`: the fixture's WAV is 1 s long, so every cue is past the end.
  - `a_guest_hotcue_edit_stays_for_the_session_only`: set slot 5, reload, and it is there. Remove the stick, plug it
    in again, and it is gone. The library is still empty.
  - `a_guest_deck_keeps_playing_when_the_stick_goes`.
  - `nothing_reaches_the_library_after_a_guest_set`:
    - load, edit cues, shift the grid, ask for stems and ask for a cover on a guest deck;
    - every library table's row count is unchanged.
  - `a_guest_track_without_a_grid_gets_one_in_memory`.
- [ ] **Step 2:** The tests fail. **Step 3:** Implement. **Step 4:** They pass, along with
  `cargo test --workspace`.
- [ ] **Step 5:** Commit: `feat(app): play guest tracks with their own cues, saving nothing`

### Task 9: Docs and render

**Files:**
- Modify: `docs/interop.md` (a "Guest sticks" section: what shows, what is never written, the session-only edits,
  that a removed stick keeps the deck playing); `docs/ui.md` (the stick node); `README.md` (one line)
- Modify: `crates/ui/tests/render.rs`: a `pro-guest` fixture with a stick node of two branches, and a guest list open

- [ ] **Step 1:** Add the fixture. Run `cargo test -p ui --test render -- --ignored` and view `pro-guest.png`.
- [ ] **Step 2:** Write the docs.
- [ ] **Step 3:** Run `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` and `cargo deny check`.
- [ ] **Step 4:** Commit: `docs: guest sticks`

---

## Hands-on (the owner)

1. Plug in OJGUEST. It shows with Serato, rekordbox, rekordbox (xml) and VirtualDJ branches, and each playlist is in
   the order made in each app.
2. Load a track from each branch:
   - its hotcues sit where they were in that app, with their colours and names;
   - the rekordbox loop and the VirtualDJ loop play;
   - rekordbox's 120 track keeps rekordbox's (wrong) 149.96 grid.
3. Edit a hotcue on a guest deck. Reload: the edit is there. Unplug and plug in again: it is gone. On the Mac, the
   stick's files are unchanged (check their modified dates).
4. Unplug while a guest track plays: it keeps playing and says the stick was removed.
5. Check that your own library gained nothing: no OJ Click tracks appear in a search of your folders.
6. The results go in `controllers/ddj-sb3/README.md`.
