# Guest stick fixture

A USB stick made on 2026-11-16 by the owner with Serato DJ Lite, rekordbox 7.2.19 and VirtualDJ 2026 on macOS,
from three synthetic click tracks (no music): `OJ Click 120.wav` (120 BPM, 1 kHz), `OJ Click 126.mp3` (126 BPM, 800 Hz)
and `OJ Click 128.mp3` (128 BPM, 600 Hz), artist "Open Jockey Fixture", album "OJ Fixtures".

Changes from the stick, so the repo carries no personal data and stays small:

- Audio is cut to its first second (WAV) or its tag and first frames (MP3). Every chunk and tag other than the
  audio data is kept, so Serato's cues (inside the files) are intact. Cue positions past the cut are expected.
- `rekordbox.xml` keeps only the three tracks and the "OJ Rekordbox A" playlist; their paths are rewritten to
  `/Music/OJ Fixtures/`.
- Left out: rekordbox's `exportLibrary.db` (the newer encrypted library, out of scope), `PIONEER/extracted`,
  `djprofile.nxs`, the `MYSETTING*.DAT` files, Serato's backups and `Library/location.sqlite`, `_Serato_Backup`,
  and VirtualDJ's `Cache`.

## What each app holds (the readers' expected values)

- **Serato** (`_Serato_`, cues in `Serato/*` tags as `Serato Markers2`; Lite saves no loops, no names):
  - crates `OJ Serato A` = 128, 120, 126; `OJ Serato B` = 126.
  - 120.wav: cue 1 10944 ms #CC0000, cue 2 26161 ms #CC8800, cue 3 44277 ms #0000CC, cue 4 35786 ms #CCCC00.
  - 126.mp3: cue 1 7954 ms #CC0000, cue 2 29418 ms #CC8800, cue 3 44451 ms #0000CC.
  - 128.mp3: cue 1 6465 ms #CC0000, cue 2 26850 ms #CC8800, cue 3 42504 ms #0000CC, cue 4 17751 ms #CCCC00.
- **rekordbox device library** (`PIONEER/rekordbox/export.pdb`, `PIONEER/USBANLZ`, files under `Contents/`):
  - playlist `OJ Rekordbox A` = 120, 128, 126 (the same as `rekordbox.xml`).
  - grids: 120.wav at **149.96 BPM** (rekordbox misread it; a real wrong grid), 126 at 126.00, 128 at 128.00.
  - cues and loops in the `ANLZ` files equal `rekordbox.xml`'s within 1 ms, with the same names and
    the same palette colours. One difference: a cue with no colour of its own has palette code 0 on the
    device, with the player's LED colour (RGB) instead of the app's default for the slot that
    `rekordbox.xml` writes. The device value is what the device reader gives: 126.mp3 A #FF0017,
    B #00C4FF, C #33FF00; 128.mp3 A #FF0017, C #33FF00 (xml: #FF376F, #45ACDB, #7DC13D for the same).
  - `rekordbox.xml` (same session):
    - 120.wav: A 5.216 s "Test 3" (180,190,4), B 22.820 s "TEst" (255,18,123), C 44.026 s "Test 2" (170,114,255),
      D a loop 30.422–32.023 s (255,140,0).
    - 128.mp3: A 6.118 s (255,55,111), B 19.243 s "Test" (224,100,27), C 53.462 s (125,193,61), memory cue 0.000 s.
    - 126.mp3: A 9.548 s (255,55,111), B 25.738 s (69,172,219), C 54.786 s (125,193,61).
- **VirtualDJ** (`VirtualDJ/database.xml` with `E:/` paths, `VirtualDJ/MyLists/OJ VDJ A.vdjfolder` with
  `/Volumes/OJGUEST/` paths; tracks under `VirtualDJ tracks/`):
  - list `OJ VDJ A` = 126, 120, 128.
  - 120.wav: cues 1 6.028934 s, 2 26.189705 s, 3 46.026667 s.
  - 128.mp3: cues 1 9.549819 s, 2 33.906735 s, 3 45.482313 s.
  - 126.mp3: cues 1 10.554195 s "Test 1", 2 28.60 s "TEst 2", 3 46.072268 s "Test 3"; loop 4 at 38.120 s, 8 beats.
  - No cue colours (VirtualDJ's defaults).
