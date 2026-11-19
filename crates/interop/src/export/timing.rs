//! Where each app shows a position that Open Jockey stores in frames.

use decode::{InfoEncoder, LeadIn};

use super::{ExportTrack, FileKind};

/// A DJ app an export is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    Serato,
    Rekordbox,
    VirtualDj,
}

/// App position − Open Jockey position, in seconds, for one file: add it to a position in Open
/// Jockey's timeline for the app's, subtract it for the reverse.
///
/// Open Jockey's frame 0 is the first sample after the gapless trim: with a Xing/Info frame it
/// trims the declared encoder delay plus the decoder's 529 frames (`LeadIn::delay_frames`,
/// 576 + 529 = 1105 for LAME and FFmpeg), and without one it trims nothing. Only an MP3 with
/// an Info frame is offset; every other file (WAV, AIFF, FLAC, M4A, an MP3 without the frame
/// or with a VBRI one) is 0 in every app. The rules rest on the OJCAL stick
/// (`tests/fixtures/ojcal`, spec `2026-11-18-mp3-offsets-design.md`, "Rules, as measured"):
/// - **Serato**: a fixed 50 ms for an MPEG-1 frame (`frame_samples` 1152; 44.1k files 01/03/04
///   show 50-51 ms, the 48k file 06 shows 50 ms, which rules out a frame count), plus one
///   MPEG-2 frame (576 / rate) when `frame_samples` is 576 (22.05k file 07: 76.1 ms, cue
///   measured 77; one data point).
/// - **rekordbox**: `delay_frames` plus one frame (`frame_samples`) for a LAME-style tag
///   (44.1k files 03/04: 51.2 ms, measured 51; 48k file 06: 47.0, measured 46; 22.05k file 07:
///   76.2, measured 76), or `delay_frames` alone for an FFmpeg (`Lavc`) tag (file 01: 25.1 ms,
///   measured 24); a "Lavf" tag (same FFmpeg writer, unmeasured) is classed like it. A tag
///   whose encoder is unreadable or another name (`None`, `Other`) is treated as LAME-style:
///   a guess, as most encoders write LAME's tag.
/// - **VirtualDJ**: 0 (the 11a import lined up with no offset).
///
/// rekordbox's values are on the same timeline as `TEMPO Inizio` and the ANLZ beat grid.
#[must_use]
pub fn app_offset_seconds(app: App, kind: FileKind, lead_in: &LeadIn, sample_rate: u32) -> f64 {
    if kind != FileKind::Mp3 || !lead_in.info_frame || sample_rate == 0 {
        return 0.0;
    }
    let rate = f64::from(sample_rate);
    match app {
        App::Serato => {
            // Also applies to an Info frame where our decoder trims nothing (delay 0 or
            // unreadable, e.g. an unknown encoder): that case is unmeasured and the +50 ms
            // is a guess.
            let mpeg2_frame = if lead_in.frame_samples == 576 {
                576.0 / rate
            } else {
                0.0
            };
            0.050 + mpeg2_frame
        }
        App::Rekordbox => {
            let extra = if lead_in.info_encoder == Some(InfoEncoder::Lavc) {
                0
            } else {
                lead_in.frame_samples
            };
            f64::from(lead_in.delay_frames + extra) / rate
        }
        App::VirtualDj => 0.0,
    }
}

/// Seconds `app` shows for Open Jockey's `frames` in `t`: the frames in seconds plus the
/// app's offset for the file (see [`app_offset_seconds`]).
#[must_use]
pub fn app_seconds(app: App, frames: f64, t: &ExportTrack) -> f64 {
    frames / f64::from(t.sample_rate)
        + app_offset_seconds(app, t.kind.clone(), &t.lead_in, t.sample_rate)
}

/// Open Jockey's seconds for a position `app` shows as `app_seconds`: the inverse of
/// [`app_seconds`], for the guest readers.
#[must_use]
pub fn ours_seconds(
    app: App,
    app_seconds: f64,
    kind: FileKind,
    lead_in: &LeadIn,
    sample_rate: u32,
) -> f64 {
    app_seconds - app_offset_seconds(app, kind, lead_in, sample_rate)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use decode::LeadIn;

    use super::*;

    fn lead(encoder: Option<InfoEncoder>, frame_samples: u32) -> LeadIn {
        LeadIn {
            gapless_header: true,
            delay_frames: 1105,
            info_frame: true,
            info_encoder: encoder,
            frame_samples,
        }
    }

    fn off(app: App, l: &LeadIn, rate: u32) -> f64 {
        app_offset_seconds(app, FileKind::Mp3, l, rate)
    }

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    fn track(kind: FileKind, gapless_header: bool) -> ExportTrack {
        let frame_samples = if matches!(kind, FileKind::Mp3) {
            1152
        } else {
            0
        };
        ExportTrack {
            id: 1,
            path: PathBuf::from("a"),
            fingerprint: [0; 32],
            size: 0,
            title: "a".into(),
            artist: None,
            album: None,
            genre: None,
            sample_rate: 44_100,
            duration_frames: None,
            bpm: None,
            first_beat_frames: None,
            key: None,
            cue_frames: None,
            hotcues: [const { None }; 8],
            kind,
            lead_in: LeadIn {
                gapless_header,
                delay_frames: if gapless_header { 1105 } else { 0 },
                // An MP3 with a gapless header has a LAME Info frame; other kinds have none.
                info_frame: gapless_header,
                info_encoder: gapless_header.then_some(decode::InfoEncoder::Lame),
                frame_samples,
            },
        }
    }

    #[test]
    fn each_rule_matches_its_spec_row() {
        let lame = lead(Some(InfoEncoder::Lame), 1152);
        near(off(App::Serato, &lame, 44_100), 0.050);
        near(off(App::Serato, &lame, 48_000), 0.050);
        let mpeg2 = lead(Some(InfoEncoder::Lame), 576);
        near(off(App::Serato, &mpeg2, 22_050), 0.050 + 576.0 / 22_050.0);
        near(
            off(App::Rekordbox, &lame, 44_100),
            (1105.0 + 1152.0) / 44_100.0,
        );
        near(
            off(App::Rekordbox, &lame, 48_000),
            (1105.0 + 1152.0) / 48_000.0,
        );
        near(
            off(App::Rekordbox, &mpeg2, 22_050),
            (1105.0 + 576.0) / 22_050.0,
        );
        let lavc = lead(Some(InfoEncoder::Lavc), 1152);
        near(off(App::Rekordbox, &lavc, 44_100), 1105.0 / 44_100.0);
        near(off(App::Serato, &lavc, 44_100), 0.050);
        let none = LeadIn::default();
        for app in [App::Serato, App::Rekordbox, App::VirtualDj] {
            near(off(app, &none, 44_100), 0.0);
            near(app_offset_seconds(app, FileKind::Wav, &lame, 44_100), 0.0);
        }
        near(off(App::VirtualDj, &lame, 44_100), 0.0);
    }

    #[test]
    fn unmeasured_headers_follow_the_spec_defaults() {
        let lame = off(App::Rekordbox, &lead(Some(InfoEncoder::Lame), 1152), 44_100);
        near(off(App::Rekordbox, &lead(None, 1152), 44_100), lame);
        near(
            off(
                App::Rekordbox,
                &lead(Some(InfoEncoder::Other), 1152),
                44_100,
            ),
            lame,
        );
        let vbri = LeadIn {
            info_frame: false,
            ..lead(None, 1152)
        };
        near(off(App::Serato, &vbri, 44_100), 0.0);
        near(off(App::Rekordbox, &vbri, 44_100), 0.0);
    }

    #[test]
    fn a_position_round_trips() {
        let rows = [
            (lead(Some(InfoEncoder::Lame), 1152), 44_100),
            (lead(Some(InfoEncoder::Lavc), 1152), 44_100),
            (lead(Some(InfoEncoder::Lame), 1152), 48_000),
            (lead(Some(InfoEncoder::Lame), 576), 22_050),
            (LeadIn::default(), 44_100),
        ];
        for (l, rate) in rows {
            for app in [App::Serato, App::Rekordbox, App::VirtualDj] {
                let mut t = track(FileKind::Mp3, true);
                t.lead_in = l;
                t.sample_rate = rate;
                for frames in [0_u64, 1, 4_410_000] {
                    let secs = app_seconds(app, frames as f64, &t);
                    let back = ours_seconds(app, secs, FileKind::Mp3, &l, rate);
                    assert_eq!((back * f64::from(rate)).round() as u64, frames);
                }
            }
        }
    }

    #[test]
    fn virtualdj_is_plain_seconds() {
        let t = track(FileKind::Mp3, true);
        assert!((app_seconds(App::VirtualDj, 44_100.0, &t) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rekordbox_mp3_with_lame_header_is_the_trimmed_delay_plus_a_frame_later() {
        let t = track(FileKind::Mp3, true);
        let want = 1.0 + (1105.0 + 1152.0) / 44_100.0;
        assert!((app_seconds(App::Rekordbox, 44_100.0, &t) - want).abs() < 1e-9);
    }

    #[test]
    fn serato_mp3_with_header_is_50_ms_later() {
        let t = track(FileKind::Mp3, true);
        assert!((app_seconds(App::Serato, 44_100.0, &t) - 1.05).abs() < 1e-9);
    }

    #[test]
    fn serato_wav_has_no_offset() {
        let t = track(FileKind::Wav, false);
        assert!((app_seconds(App::Serato, 44_100.0, &t) - 1.0).abs() < 1e-9);
    }

    // The OJCAL stick (see tests/fixtures/ojcal/README.md). In this module rather than in
    // tests/ojcal.rs because the Serato tag reader and the ANLZ reader are crate-private.

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ojcal");

    /// File, sample rate, Serato hot cue 1 (ms), Serato anchor checked, rekordbox `Inizio`
    /// (s), ANLZ folder.
    #[allow(clippy::type_complexity)]
    const OJCAL: [(&str, u32, Option<f64>, bool, f64, &str); 8] = [
        (
            "OJ Cal 00 reference 44k.wav",
            44_100,
            None,
            true,
            0.001,
            "P07A",
        ),
        (
            "OJ Cal 01 ffmpeg 44k CBR.mp3",
            44_100,
            Some(51.0),
            true,
            0.024,
            "P072",
        ),
        (
            "OJ Cal 02 ffmpeg 44k no-Info.mp3",
            44_100,
            Some(27.0),
            false,
            0.024,
            "P01E",
        ),
        (
            "OJ Cal 03 lame 44k CBR.mp3",
            44_100,
            Some(51.0),
            true,
            0.051,
            "P028",
        ),
        (
            "OJ Cal 04 lame 44k VBR.mp3",
            44_100,
            None,
            true,
            0.051,
            "P004",
        ),
        (
            "OJ Cal 05 lame 44k no-Info.mp3",
            44_100,
            None,
            false,
            0.024,
            "P020",
        ),
        (
            "OJ Cal 06 lame 48k CBR.mp3",
            48_000,
            Some(50.0),
            true,
            0.046,
            "P009",
        ),
        (
            "OJ Cal 07 lame 22k CBR.mp3",
            22_050,
            Some(77.0),
            false,
            0.076,
            "P051",
        ),
    ];

    /// The click's frame in Open Jockey's decode: the first sample over 0.1, less the one frame
    /// the click's sine takes to rise past it (the README's measured onsets are 1 and 1106).
    /// Chosen over a lower threshold so the measurement matches `tests/ojcal.rs`.
    fn click_frame(path: &std::path::Path) -> f64 {
        let track = decode::decode_file(path).unwrap().track;
        let onset = (0..track.frames())
            .find(|&i| track.frame(i).iter().any(|s| s.abs() > 0.1))
            .expect("a click");
        (onset - 1) as f64
    }

    fn inizio(xml: &str, stem: &str) -> f64 {
        let row = xml.split("<TRACK ").find(|r| r.contains(stem)).unwrap();
        let at = row.find("Inizio=\"").unwrap() + "Inizio=\"".len();
        row[at..].split('"').next().unwrap().parse().unwrap()
    }

    #[test]
    fn ojcal_values_land_on_the_clicks() {
        use crate::export::serato::markers::{Marker, read_markers2};
        use crate::export::serato::tags::{read_id3, serato_frames};

        let dir = std::path::Path::new(FIXTURE);
        let xml = std::fs::read_to_string(dir.join("rekordbox.xml")).unwrap();
        let mut worst = [0.0_f64; 3];
        for (name, rate, cue_ms, anchor, want_inizio, anlz_dir) in OJCAL {
            let path = dir.join(name);
            let kind = FileKind::of(&path);
            let lead_in = decode::lead_in(&path).unwrap();
            let ours = click_frame(&path) / f64::from(rate);
            let off = |app| app_offset_seconds(app, kind.clone(), &lead_in, rate);

            // rekordbox: Inizio and the ANLZ first beat, +-1.5 ms (ruling R1).
            let rb = (ours + off(App::Rekordbox) - want_inizio).abs() * 1000.0;
            assert_eq!(inizio(&xml, &name[..name.len() - 4]), want_inizio, "{name}");
            let folder = dir.join("PIONEER/USBANLZ").join(anlz_dir);
            let sub = std::fs::read_dir(&folder)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let beat = crate::guest::anlz::read_anlz(&sub.join("ANLZ0000.DAT"), None)
                .unwrap()
                .grid
                .expect("a beat grid")
                .0;
            let rb_anlz = (beat - want_inizio).abs() * 1000.0;
            println!("{name}: rekordbox xml {rb:.2} ms, ANLZ vs Inizio {rb_anlz:.2} ms");
            assert!(rb <= 1.5, "{name}: rekordbox {rb:.2} ms");
            assert!(
                rb_anlz <= 1.5,
                "{name}: ANLZ beat {beat} vs Inizio {want_inizio}"
            );
            worst[0] = worst[0].max(rb);

            // Serato hot cue 1 (ms), +-2 ms, and the tempo-map anchor (one second in), +-1 ms.
            let tag = read_id3(&path, &kind).unwrap().expect("an ID3 tag");
            let frames = serato_frames(&tag);
            if let Some(want) = cue_ms {
                let markers = read_markers2(&frames["Serato Markers2"]);
                let Some(Marker::Cue { ms, .. }) = markers
                    .into_iter()
                    .find(|m| matches!(m, Marker::Cue { index: 0, .. }))
                else {
                    panic!("{name}: no hot cue 1");
                };
                assert_eq!(f64::from(ms), want, "{name}");
                let err = (ours * 1000.0 + off(App::Serato) * 1000.0 - want).abs();
                println!("{name}: serato cue {err:.2} ms");
                assert!(err <= 2.0, "{name}: serato cue {err:.2} ms");
                worst[1] = worst[1].max(err);
            }
            if anchor {
                let map = &frames["Serato Tempo Map"];
                let at = f64::from_be_bytes(map[4..12].try_into().unwrap());
                let err = (at - 1.0 - ours - off(App::Serato)).abs() * 1000.0;
                println!("{name}: serato anchor {err:.2} ms");
                assert!(err <= 1.0, "{name}: serato anchor {err:.2} ms");
                worst[2] = worst[2].max(err);
            }
        }
        println!(
            "worst: rekordbox {:.2}, cue {:.2}, anchor {:.2}",
            worst[0], worst[1], worst[2]
        );
    }
}
