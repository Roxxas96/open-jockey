# Exporting to a USB stick

Open Jockey copies playlists onto a USB stick so Serato, Rekordbox and VirtualDJ on another
computer show them in order, with their hotcues and saved loops. One export writes what all
three apps need; there is no choice of app. Your music files are never changed: the Serato
tags go into the copies on the stick only.

**Checked by hand.** The owner opened an exported stick in Serato DJ Lite, Rekordbox and
VirtualDJ 2026 on macOS (2026-11-10 and 11):

- The playlists show in order in all three, and hotcue positions are exact in all three. The
  timing constants below are confirmed: a Serato MP3 with an Info frame is 1152 frames
  later, the others are unshifted.
- Exporting again copies nothing new and shows a hotcue that was moved; a playlist that is
  unticked disappears from all three.
- Serato shows a track that is repeated in a playlist once: that is Serato's own limit (it
  keeps a track once per crate), and the export says so in a note. Saved loops are written,
  but Serato DJ Lite has no saved loops; they are for Serato DJ Pro, which has not been
  checked.
- Rekordbox imports the playlists through the XML, with the steps in
  `IMPORT TO REKORDBOX.txt` at the top of the stick (they are also shown when an export ends).
- VirtualDJ shows the playlists and cues correctly. It offered a one-time "database is
  corrupted, fix it?" because of the drive letter; an export now keeps the letter VirtualDJ
  uses: exporting again no longer asks, and no track is listed twice.

Not verified yet: a CDJ (not exported), another computer or Windows, and Serato DJ Pro's saved
loops.

To export, see [library.md](library.md#exporting-to-a-usb-stick) for the command line, and
[ui.md](ui.md#the-settings-window) for Settings, USB export in the window.

## What goes on the stick

```text
<stick>/
  Open Jockey/
    Music/<artist>/<file>       the copies, one per track
    rekordbox.xml               for Rekordbox's XML import
    README.txt                  how to open the stick in each app
    export.toml                 the manifest: what Open Jockey wrote
  _Serato_/
    Subcrates/<playlist>.crate  one crate per playlist
    database V2                 our tracks, merged in
  VirtualDJ/
    database.xml                our tracks, merged in
    MyLists/<playlist>.vdjfolder
  IMPORT TO REKORDBOX.txt      the Rekordbox import steps, rewritten on each export
```

- **The copies:** each track is copied once, under `Open Jockey/Music/<artist>/<file name>`
  (`Unknown Artist` with no artist tag). Names are made safe for FAT32 and exFAT: characters
  Windows forbids become `_`, device names such as `CON` get a `_`, and long names are cut.
  Two files that would get the same name (ignoring case) become `name (2).mp3` and so on.
- **The manifest** (`export.toml`) lists the playlists, crates, VirtualDJ lists and copies
  Open Jockey wrote. On the stick, Open Jockey changes or deletes only what the manifest
  lists, and inside Serato's `database V2` and VirtualDJ's `database.xml` only the entries
  that point into `Open Jockey/`. What other apps put on the stick stays as it is.
- **Exporting again** copies only new or changed tracks (by fingerprint), rewrites
  the Serato tags of a copy whose hotcues changed, and deletes the copies, crates and lists of
  playlists no longer exported. The window ticks the playlists the stick already holds.
- **Before anything is written** the export checks the free space (with a 32 MiB margin); a
  stick that is too small, or whose free space the system will not tell, is refused. A stick that is, holds or is inside one of the library's
  folders is refused too, as is one whose manifest, `database V2` or `database.xml` cannot be
  read.
- **An interrupted export** (stick pulled, app closed) leaves the stick usable: every file is
  written under a temporary `.ojpart` name and renamed into place, and the next export
  removes what is left over. Before the first copy the manifest already lists the crates,
  lists and copies the export is about to write, so the next export knows them as its own.

## What each app gets

- **Serato DJ:** a crate per playlist, in order, under the stick's name. The hotcues (with
  colour and name), the saved loops and the beatgrid are in the `Serato Markers2` and
  `Serato BeatGrid` tags of the copies. Serato keeps cues inside audio files, so only MP3 and
  WAV copies carry them; other types are copied without Serato cues, and the summary counts
  them.
- **Rekordbox:** `Open Jockey/rekordbox.xml`, the documented collection format. Each track has
  its grid (`TEMPO`), its main cue as a memory cue, and its hotcues and saved loops
  (`POSITION_MARK`, with colour and name), and each playlist is in order. Rekordbox needs one
  step: Preferences, Advanced, Database, rekordbox xml, choose the file, then import the
  playlists from the "rekordbox xml" view. On a computer where the stick has another name or
  drive letter, Rekordbox shows the tracks as missing: use Relocate on `Open Jockey/Music`.
  CDJs do not read this file; a CDJ export is not planned yet.
- **VirtualDJ:** the drive's own `VirtualDJ/database.xml`, which VirtualDJ reads from an
  external drive, with the hotcues (colour and name) and saved loops, and a list per playlist
  under `VirtualDJ/MyLists`. VirtualDJ keeps a loop's length in beats, so a saved loop on a
  track with no BPM is left out, and the export says how many. The drive letter in the
  database is the one VirtualDJ assigns to the volume, so VirtualDJ may offer a one-time fix
  ("the drive's database is corrupted") on a new computer; say yes, it only updates the letter.
  An export reuses the letter already on the stick (`D:` when there is none yet).

`README.txt` on the stick repeats these steps for whoever plugs it in.

## Timing

Open Jockey stores positions in frames, counted from the first sample after the gapless trim
(an MP3's Info frame and encoder delay are skipped). Each app counts from its own start, so
`export/timing.rs` in the interop crate converts a position for each app:

| App | Shift | Constant |
| --- | --- | --- |
| VirtualDJ | none (the 11a import lined up with no offset) | none |
| Rekordbox | none | none |
| Serato, MP3 with an Info frame | one MP3 frame (1152 samples, about 26 ms) later | `SERATO_INFO_FRAME_FRAMES` |
| Serato, WAV or MP3 without one | none | none |

These values were confirmed in the hands-on with a click track: a 60 s file with a
4 ms, 2 kHz click every half second (120 BPM), as MP3 and as WAV, with hotcue 1 on the click at
8.0 s and a 4-beat saved loop at 16.0 s. Exported and opened in each app at full zoom, hotcue 1
must sit on the click's start. If an app ever shows a shift, change that app's constant in
`crates/interop/src/export/timing.rs` and repeat the click-track check.

Two things the hands-on checked or still has to check:

- **A track twice in one playlist:** Serato shows it once (its limit); Rekordbox and
  VirtualDJ keep both entries.
- **VirtualDJ lists on another computer (not checked yet):** a `.vdjfolder` names each track by
  its absolute path, so the stick is plugged into a computer where it mounts under another name
  (or drive letter), and VirtualDJ must still find the tracks of each list.

## Guest sticks

A USB stick that another DJ's Serato, rekordbox or VirtualDJ exported shows in the browser,
and its tracks play with that app's cues. Only a volume with an app's marker counts
(`_Serato_/`, `PIONEER/rekordbox/export.pdb`, a `rekordbox.xml` at the root, `VirtualDJ/`),
and a volume that holds one of your own library folders is never a guest stick. It shows
when it is plugged in and goes when it is removed.

**What shows.** The stick's node replaces the drive's plain row. It has one branch per app
("Serato", "rekordbox", "rekordbox (xml)", "VirtualDJ"), each with an "All tracks" list
first (every track that app knows, in its order, so a track in no playlist or crate still
loads), then its playlists or crates in the order that app has them (nested folders flatten
to "Folder / Playlist"), and a "Files" child for browsing the stick's folders (those rows do
not load). A source the app cannot read shows its problem as the branch's note.

**What is never written.** Nothing on the stick is changed: it is only read, with no cache
or index file. Nothing of it reaches your library either: no track, cue, grid, cover, stems
row or playlist entry. These are refused: dropping or adding a guest track to a playlist
("Guest tracks can't be added to your playlists"), and deleting (moving to the Trash) a
guest stick's rows or files ("A guest stick is never changed"). The export window never
offers a guest stick as a target; a stick Open Jockey exported to before (it holds Open
Jockey's manifest) is still offered.

**How a track plays.** It loads with its app's hotcues, saved loops and grid. Cues past the
end of the file are dropped, with a message. Serato's grid comes from each file's `Serato
Tempo Map` tag (or the older `Serato BeatGrid`). A track with no grid gets one detected in
memory only, and so does a track whose app gives a BPM but no first beat (VirtualDJ) or a
BPM outside 40 to 300. Cover lookup, cover removal and stems are not available for guest
tracks. Guest rows show no length and no key (the stick gives none), so the list's footer
shows its track count only.

**Edits last for the session.** Hotcue, label and grid edits on a guest deck are kept in
memory, so a reload shows them, but they are lost when the stick is unplugged.

**Removing the stick.** A deck playing a guest track keeps playing what it has decoded, and
its title shows "(stick removed)". Loading from a stick that has gone fails with "Stick
{name} is no longer plugged in".

Not covered: CDJ playback, and rekordbox's encrypted `exportLibrary.db`.

## Known limits

- Not verified: another computer or Windows, and Serato DJ Pro's saved loops (see the top of
  this page).
- One level of playlists: folders are not exported.
- No CDJ export (`PIONEER/` with `export.pdb`): there is no CDJ to verify it on.
- Guest sticks are read only; see [Guest sticks](#guest-sticks) for what they do not do.
