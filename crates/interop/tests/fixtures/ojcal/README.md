# OJCAL calibration stick fixture

A USB stick (volume `OJCAL`) made on 2026-11-17 and 2026-11-18 by the owner with Serato DJ Lite and rekordbox 7.2.19
on macOS, from eight synthetic click files (no music). Each carries a 2 ms, 2 kHz click every 0.5 s (120 BPM), 30 s
long, written so that a click starts at exactly 0 s in the source PCM. It pins how far Serato's and rekordbox's
timelines sit from Open Jockey's decode for each kind of MP3 header (design:
`docs/superpowers/specs/2026-11-18-mp3-offsets-design.md`).

Changes from the stick, so the repo carries no personal data and stays small:

- Audio is cut to its first 2.5 s, on a frame boundary:
  - MP3s keep their ID3v2 tag whole (Serato's `GEOB` tags live there), then the first `ceil(2.5 s / frame) + 1`
    frames (the Xing/Info frame, where there is one, holds no audio). None has an ID3v1 tag.
  - The WAV keeps every RIFF chunk (`fmt `, `data`, `id3 `, `LIST`), with `data` cut to 110,250 frames and the
    chunk and RIFF sizes fixed up.
  - Every Serato cue and tempo-map anchor below sits at or before 1.07 s, so all are inside the cut.
  - The cut files keep their Xing/Info frames unchanged, so the frame counts there (and the sizes in `export.pdb`
    and `rekordbox.xml`) are the full files'. `decode::decode_file` reports the cut MP3s with a header as
    truncated; the onsets don't depend on it.
- `rekordbox.xml` keeps only the eight `OJ Cal` `TRACK` rows, unchanged, and the `Test` playlist that holds them
  (the collection's and ROOT's counts are fixed up). Its paths stay `/Volumes/OJCAL/…`. Line endings are LF
  (the stick's were CRLF).
- Kept byte for byte: `PIONEER/rekordbox/export.pdb` and `exportExt.pdb`, `PIONEER/USBANLZ/**`, and
  `_Serato_/database V2`, `neworder.pref`, `Subcrates/Test.crate`.
- Left out: `exportLibrary.db*`, `PIONEER/*.DAT`, `djprofile.nxs`, `PIONEER/extracted`, `_Serato_/Library`,
  `_Serato_/Export Backups` and `_Serato_Backup`.

## The files

| File | Header (encoder string, delay) | MPEG, rate | Frame samples |
|---|---|---|---|
| OJ Cal 00 reference 44k.wav | — | WAV 16-bit 44.1 kHz | — |
| OJ Cal 01 ffmpeg 44k CBR.mp3 | Info, `Lavc63.1.`, 576 | MPEG-1, 44.1 kHz, 320 kb/s | 1152 |
| OJ Cal 02 ffmpeg 44k no-Info.mp3 | none | MPEG-1, 44.1 kHz, 320 kb/s | 1152 |
| OJ Cal 03 lame 44k CBR.mp3 | Info, `LAME4.0`, 576 | MPEG-1, 44.1 kHz, 320 kb/s | 1152 |
| OJ Cal 04 lame 44k VBR.mp3 | Xing, `LAME4.0`, 576 | MPEG-1, 44.1 kHz, VBR | 1152 |
| OJ Cal 05 lame 44k no-Info.mp3 | none | MPEG-1, 44.1 kHz, 320 kb/s | 1152 |
| OJ Cal 06 lame 48k CBR.mp3 | Info, `LAME4.0`, 576 | MPEG-1, 48 kHz, 320 kb/s | 1152 |
| OJ Cal 07 lame 22k CBR.mp3 | Info, `LAME4.0`, 576 | MPEG-2, 22.05 kHz, 128 kb/s | 576 |

## Measured values

Onset is the first frame of `decode::decode_file` with a sample over 0.1 in absolute value
(`tests/ojcal.rs`). Open Jockey trims 576 + 529 = 1105 frames when there is a Xing/Info frame, and nothing otherwise.
The click's first sample (a sine starting at 0) is below 0.1, so every onset is 1 frame past the click: frame 1 for the
trimmed files and the WAV, and 1105 + 1 = 1106 for 02 and 05, as the design's rules table expects.

| File | Our onset (frames) | Serato tempo map anchor, BPM | Serato hot cue 1 | rekordbox `TEMPO Inizio` | rekordbox ANLZ first beat |
|---|---|---|---|---|---|
| 00 | 1 | 1.000000 s, 120.000000 | — | 0.001 s | 1 ms |
| 01 | 1 | 1.050000 s, 120.000005 | 51 ms | 0.024 s | 24 ms |
| 02 | 1106 | 0.010000 s, 30.000000 (misread) | 27 ms | 0.024 s | 24 ms |
| 03 | 1 | 1.050000 s, 120.000005 | 51 ms | 0.051 s | 51 ms |
| 04 | 1 | 1.050000 s, 120.000005 | — | 0.051 s | 51 ms |
| 05 | 1106 | 0.010000 s, 30.000000 (misread) | — | 0.024 s | 24 ms |
| 06 | 1 | 1.050000 s, 120.000005 | 50 ms | 0.046 s | 46 ms |
| 07 | 1 | 1.068614 s, 119.979697 | 77 ms | 0.076 s | 76 ms |

- **Serato**: the `Serato Tempo Map` GEOB tag holds a big-endian `f64` anchor at byte 4 and an `f64` BPM at byte 24.
  The WAV's anchor of 1.0 s is a beat (the click at 1.0 s). Hot cue 1 was placed by eye on the first click, with no
  snapping, on 2026-11-18, and is read from `Serato Markers2` (ms resolution). Files 00, 04 and 05 have no cue.
- **rekordbox**: `TEMPO Inizio` is from `rekordbox.xml` (ms resolution). The ANLZ first beat is the first `PQTZ`
  entry in each `ANLZ0000.DAT` (all at 120.00 BPM); it equals `Inizio` on every file, so the device export and the
  XML share one timeline. The ANLZ files hold no cues. The ANLZ folders map to files as P07A 00, P072 01, P01E 02,
  P028 03, P004 04, P020 05, P009 06, P051 07.

## Hands-on: checking an export against Serato and rekordbox

Do this after a change to `export/timing.rs`, with the OJCAL files in a library:

1. In Open Jockey, put hot cue 1 on the first click of each OJ Cal track (the click is at the very start
   of the decode, apart from files 02 and 05, where it sits about 25 ms in), then export a playlist of the
   eight to a stick.
2. Open the stick in Serato DJ Lite, and its `Open Jockey/rekordbox.xml` in rekordbox (see
   `IMPORT TO REKORDBOX.txt`). Zoom right in. In both, cue 1 sits on the click start for 01, 03, 06 and 02.
3. Plug the exported stick back in as a guest (a stick copy of `rekordbox.xml` at its root for rekordbox).
   Open Jockey's cues sit on the clicks.
4. Record what you see in `controllers/ddj-sb3/README.md`.

`cargo test -p interop --test ojcal` checks the same thing without the apps: the exported Serato cue
milliseconds and `Inizio` equal the values in the table above (Serato within 2 ms, rekordbox within
1.5 ms), and reading the exported stick back returns the original frames.
