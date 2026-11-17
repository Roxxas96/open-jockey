# MP3 positions in Serato and rekordbox — design

Approved by the owner on 2026-11-18, following the OJCAL calibration.

## Intent

Cues, saved loops and beat grids on MP3s must land in the same place in Open Jockey and in the other apps, in both directions:

- a friend's Serato or rekordbox stick plugged in (guest sticks, plan 11b);
- a stick exported for a friend (plan 11c);
- rekordbox.xml read from a stick.

Today Serato MP3 positions are about 24 ms early (Open Jockey shifts them 26 ms; Serato shifts 50 ms), and rekordbox MP3 positions are not shifted at all, so they are 24–51 ms out. At 128 BPM, 50 ms is about a tenth of a beat, which is audible on a hot cue drop. The 11c hands-on called the export positions "exact" only because ~25 ms is too small to see at normal zoom.

## Goals

- One function works out, per file, how far each app's timeline sits from Open Jockey's. Export and every guest reader use it, so a position exported then read back returns to the same frame.
- The shift is derived from the file's own header: whether it has a Xing/Info frame, which encoder wrote that frame, the encoder delay, the sample rate and the MPEG version.
- Tests pin the shift against real Serato and rekordbox output for every OJCAL file.

## Non-goals

- VirtualDJ is unchanged. Its positions already matched, both in the 11a import and in the 11c export.
- WAV, AIFF, FLAC and M4A are unchanged. The WAV measured 0 ms in Serato and 1 ms in rekordbox, which is rekordbox's onset bias.
- Positions already stored in the owner's library are not migrated. Open Jockey's own frame positions are not wrong; only the conversion to and from the other apps changes.
- CDJ and Engine DJ are out of scope.

## Anti-goals

- **No fitted constant without a model behind it.** Each rule must be explained by the header fields it reads, and its test must cover every OJCAL file.
- **No change to how Open Jockey decodes.** Frame 0 stays wherever the decoder puts it today. All correction lives in the conversion.

## The measurements

The OJCAL stick was made on 2026-11-17 with Serato DJ Lite and rekordbox 7.2.19 on macOS.

Each file carries a 2 ms, 2 kHz click every 0.5 s (120 BPM), 30 s long, written so that a click starts at exactly 0 s in the source PCM:

| File | Header (encoder string, delay) | Serato tempo map anchor | rekordbox `TEMPO Inizio` |
|---|---|---|---|
| OJ Cal 00 reference 44k.wav | — | 1.000000 s (a beat at 1.0) | 0.001 |
| OJ Cal 01 ffmpeg 44k CBR.mp3 | Info, `Lavc63.1.`, 576 | 1.050000 s | 0.024 |
| OJ Cal 02 ffmpeg 44k no-Info.mp3 | none | 0.010000 s, but BPM read as 30 | 0.024 |
| OJ Cal 03 lame 44k CBR.mp3 | Info, `LAME4.0`, 576 | 1.050000 s | 0.051 |
| OJ Cal 04 lame 44k VBR.mp3 | Xing, `LAME4.0`, 576 | 1.050000 s | 0.051 |
| OJ Cal 05 lame 44k no-Info.mp3 | none | 0.010000 s, but BPM read as 30 | 0.024 |
| OJ Cal 06 lame 48k CBR.mp3 | Info, `LAME4.0`, 576 | 1.050000 s | 0.046 |
| OJ Cal 07 lame 22k CBR.mp3 (MPEG-2) | Info, `LAME4.0`, 576 | 1.068614 s, BPM 119.98 | 0.076 |

Notes on the table:

- **Serato's tempo map** is the `Serato Tempo Map` GEOB tag: a big-endian f64 anchor at byte 4 and an f64 BPM at byte 24. The owner confirmed that Serato shows 30 BPM for files 02 and 05. Their anchor is still the onset Serato found.
- **rekordbox's values** have millisecond resolution. The device export's ANLZ beat grid is in the source copy too.
- **The source copy** of the files and the apps' data is at `/private/tmp/claude-501/-Users-gomez-Code-dj-software/a40742e0-d0f8-4e69-aefc-30c1d17256a8/scratchpad/ojcal-back`. Its `rekordbox.xml` holds the owner's whole collection; it must never be committed, and only the OJ Cal rows may be copied out.
- **Serato hot cues.** The owner placed hot cue 1 by eye on the first click, with no snapping, on 2026-11-18. Serato saved these positions in `Serato Markers2`, in ms:

  | File | Hot cue 1 |
  |---|---|
  | 01 ffmpeg 44k | 51 |
  | 03 LAME 44k | 51 |
  | 06 LAME 48k | 50 |
  | 07 LAME 22k | 77 |
  | 02 ffmpeg 44k, no Info | 27 |

  The cues agree with the tempo-map anchors to within 1 ms on 01, 03 and 06. So Serato's cues and grid share one timeline.

  On 02, the cue at 27 ms shows that the tempo-map anchor of 10 ms was junk from the misread tempo.

**Rules, as measured.** "Ours" is Open Jockey's decode. It trims the declared encoder delay plus the decoder delay, 576 + 529 = 1105 frames, when the file has a Xing/Info frame. `decode::lead_in` reports `delay_frames` = 1105. It trims nothing when there is no such frame.

| Case | Serato | rekordbox |
|---|---|---|
| Xing/Info frame, MPEG-1 (32/44.1/48 kHz) | ours + 50 ms, fixed time (44.1k: 50–51 ms; 48k: 50 ms, where 2257 frames would give 47) | ours + delay frames + one frame (1152) for a LAME-style tag (44.1k: 51.2 ms, measured 51; 48k: 47.0, measured 46) |
| The same, `Lavc`-written tag | as above, 50 ms | ours + delay frames only (25.1 ms, measured 24) |
| Xing/Info frame, MPEG-2/2.5 (≤ 24 kHz) | ours + 50 ms + one MPEG-2 frame (576 samples): 76.1 ms, measured 77. One data point | ours + delay frames + one frame (576): 76.2 ms, measured 76 |
| No Xing/Info frame | ours: no shift. Serato's cue at 27 ms matches our untrimmed click at about 25 ms, within the error of a cue placed by eye | ours: no shift (measured 24, against our click at about 25 ms) |

The builder confirms "ours" for each file by measuring the click onset in `decode::decode_file`. If a measurement contradicts a row, the measurement wins, and the row is corrected in this document.

## Decided

- **Owner (2026-11-18):** the rules apply to import (guest Serato, guest rekordbox device and XML) and to export (Serato and rekordbox), in one change.
- **Owner:** the last step is a hands-on check. Export the OJCAL files, then look at a hot cue on a click in Serato and in rekordbox, zoomed right in.
- **My call:** the shift is computed from the file's header, through one shared function that replaces `export::timing::SERATO_INFO_FRAME_FRAMES` and the copy of it in `guest/serato.rs`.
  - The function needs the Info/Xing encoder string and the MPEG version, which `decode::LeadIn` doesn't give today. Either extend `LeadIn`, or read the first frame in interop, whichever fits the code better.
  - A header that can't be read means no shift, which is today's behaviour for unknown files.
- **My call:** MPEG-2 and 2.5 MP3s (sample rates of 24 kHz and below) use the same rules, with one frame being 576 samples. They are marked as resting on one shaky data point: Serato's BPM there was 119.98, so its anchor may be off. The 22k file is tested against rekordbox's value and against Serato's hot cue, not against Serato's tempo-map anchor.
- **My call:** a stick exported before this change gets its MP3 positions moved on the next export. The export's change detection must treat that as a changed track and rewrite it. The old positions were the wrong ones.

- **My call (from the builder check): how the shift is modelled.**
  - Each app's offset and Open Jockey's own offset are both expressed in header terms, and the shift is their difference.
  - Open Jockey's offset is what `decode_file` really trims: the declared encoder delay when there is a header, and nothing otherwise. The builder confirms it with the measured onsets.
  - The encoder delay is a parameter, not the constant 576. In rekordbox, the encoder-plus-decoder delay is the declared delay plus 529.
- **My call: an Info/Xing frame from an encoder other than Lavc** is treated like a LAME one. Most encoders write the LAME-style tag. This is a guess and is marked as one in the code.
- **My call: Serato's MPEG-1 shift is a fixed 50 ms.** The 48 kHz cue rules out a frame count. The MPEG-2 row rests on one file.
- **My call: ANLZ and XML.** The builder checks whether the rekordbox device export's ANLZ beat grid matches `TEMPO Inizio` for the OJ Cal files. If they differ, the device reader and the XML reader get separate shifts.
- **My call: rounding.** Seconds are converted to frames by rounding to the nearest frame. The Serato round trip must be exact. A rekordbox round trip only has to land within its 1 ms resolution.
- **My call: re-export.** The builder picks how changed MP3 positions trigger a rewrite: by comparing the written bytes, or with a version number for Serato and rekordbox in the stick's export state. VirtualDJ files must not be rewritten.

## Tests

- **A fixture of the OJCAL files.** Cut each to its first 2–3 s, keeping the ID3 tags (Serato's tags live there) and the header frames intact. Copy only the OJ Cal values from rekordbox.xml and the ANLZ files into a README. Fixture files are byte-exact, as in the guest fixture.
- **For each file and each app:** Open Jockey's measured click onset, converted to the app's seconds, must land on the app's measured value. The tolerance is ±1 ms for rekordbox (its resolution) and ±1 ms for Serato. Serato is checked against its hot cues: ±2 ms, since they were placed by eye with ms resolution. Its tempo-map anchors are checked at ±1 ms where the BPM was read correctly, which excludes 02, 05 and 07.
- **Round trip:** for each fixture header type, Open Jockey frames → app seconds → Open Jockey frames returns the same frame.
- **Existing tests:** the existing Serato and rekordbox export and guest tests are updated to the new values. The guest fixture README's measured offsets section is corrected.

## Left to the builder

- Where the shared function lives. It could stay in `export/timing.rs`, renamed, or move to a module both export and guest use.
- How the header is read.
- How the fixture is cut.
- Whether the ±1 ms tolerance needs widening for one file. If it does, say why in the test and in the report; never widen it silently.

## Verification

- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` and `cargo deny check` pass.
- All CI jobs are green on the PR.
- The owner's hands-on check, recorded in `controllers/ddj-sb3/README.md`.
