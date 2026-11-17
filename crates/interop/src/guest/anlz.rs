//! rekordbox's `ANLZ` analysis files (`.DAT`, `.EXT`): the beat grid and the cues of a track.
//!
//! Big-endian throughout: a `PMAI` file header, then sections of a four-letter tag, the length
//! of the section's header and the length of the whole section. A tag this reader does not
//! know is skipped by its length. Every length is checked against the bytes that remain.

use std::path::Path;

use super::GuestCue;
use crate::virtualdj::InteropError;

/// The largest analysis file read; `.EXT` files hold waveforms and run to a few hundred KiB.
const MAX_ANLZ_BYTES: u64 = 64 * 1024 * 1024;
/// Slots of a guest hotcue.
const SLOTS: u32 = 8;

/// What a track's analysis files say.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct AnlzData {
    /// The first beat in seconds and the tempo in BPM.
    pub grid: Option<(f64, f64)>,
    pub cues: Vec<GuestCue>,
    /// The first memory cue, in seconds.
    pub main_cue: Option<f64>,
}

/// One cue of a `PCPT` or `PCP2` entry.
#[derive(Debug, Clone, PartialEq)]
struct RawCue {
    /// 0 for a memory cue, 1..=8 for hot cue A..H.
    hot_cue: u32,
    is_loop: bool,
    start_ms: u32,
    end_ms: u32,
    color: Option<u32>,
    comment: Option<String>,
}

/// The cue lists of one file: hot cues and memory cues, from `PCOB` and from `PCO2`.
#[derive(Debug, Default)]
struct Parsed {
    grid: Option<(f64, f64)>,
    basic: Vec<RawCue>,
    extended: Vec<RawCue>,
}

/// Reads the analysis of a track: the `.DAT`, and the `.EXT` when there is one. The `.EXT`'s
/// extended cues (colour, comment) win over the `.DAT`'s plain ones.
pub(crate) fn read_anlz(dat: &Path, ext: Option<&Path>) -> Result<AnlzData, InteropError> {
    let dat_bytes = read_file(dat)?;
    let ext_bytes = ext.map(read_file).transpose()?;
    Ok(combine(&dat_bytes, ext_bytes.as_deref()))
}

fn read_file(path: &Path) -> Result<Vec<u8>, InteropError> {
    let io = |source| InteropError::Io {
        path: path.to_path_buf(),
        source,
    };
    let meta = std::fs::metadata(path).map_err(io)?;
    if meta.len() > MAX_ANLZ_BYTES {
        return Err(InteropError::Format {
            path: path.to_path_buf(),
            why: "the analysis file is too large".into(),
        });
    }
    std::fs::read(path).map_err(io)
}

/// The track's data from the bytes of its `.DAT` and `.EXT`.
fn combine(dat: &[u8], ext: Option<&[u8]>) -> AnlzData {
    let dat = parse(dat);
    let ext = ext.map(parse).unwrap_or_default();
    let grid = dat.grid.or(ext.grid);
    // Per kind of cue, the extended list wins when it holds any.
    let pick = |hot: bool| -> Vec<RawCue> {
        let of_kind = |list: &[RawCue]| -> Vec<RawCue> {
            list.iter()
                .filter(|c| (c.hot_cue > 0) == hot)
                .cloned()
                .collect()
        };
        let extended = [of_kind(&ext.extended), of_kind(&dat.extended)].concat();
        if extended.is_empty() {
            [of_kind(&dat.basic), of_kind(&ext.basic)].concat()
        } else {
            extended
        }
    };
    let mut cues: Vec<GuestCue> = Vec::new();
    for cue in pick(true) {
        let slot = cue.hot_cue - 1;
        if slot >= SLOTS {
            continue;
        }
        let slot = u8::try_from(slot).unwrap_or(0);
        if cues.iter().any(|c| c.slot == slot) {
            continue;
        }
        let loop_seconds = (cue.is_loop && cue.end_ms > cue.start_ms)
            .then(|| f64::from(cue.end_ms - cue.start_ms) / 1000.0);
        cues.push(GuestCue {
            slot,
            seconds: f64::from(cue.start_ms) / 1000.0,
            loop_seconds,
            color: cue.color,
            name: cue.comment,
        });
    }
    cues.sort_by_key(|c| c.slot);
    // Only a plain memory cue is a main cue; a memory loop is not.
    let main_cue = pick(false)
        .into_iter()
        .find(|c| !c.is_loop)
        .map(|c| f64::from(c.start_ms) / 1000.0);
    AnlzData {
        grid,
        cues,
        main_cue,
    }
}

/// The four tag bytes at `at`.
fn tag_at(b: &[u8], at: usize) -> Option<&[u8]> {
    b.get(at..at.checked_add(4)?)
}

/// The big-endian word `off` bytes after `at`.
fn be32_at(b: &[u8], at: usize, off: usize) -> Option<u32> {
    be32(b, at.checked_add(off)?)
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Walks the sections of one file. A damaged section ends the walk and keeps what was read.
fn parse(bytes: &[u8]) -> Parsed {
    let mut out = Parsed::default();
    if bytes.get(..4) != Some(b"PMAI") {
        return out;
    }
    let Some(mut at) = be32(bytes, 4).map(|n| n as usize) else {
        return out;
    };
    while let (Some(tag), Some(head), Some(total)) = (
        tag_at(bytes, at),
        be32_at(bytes, at, 4),
        be32_at(bytes, at, 8),
    ) {
        let (head, total) = (head as usize, total as usize);
        let Some(end) = at.checked_add(total).filter(|&e| e <= bytes.len()) else {
            break;
        };
        if head < 12 || total < head {
            break;
        }
        let section = &bytes[at..end];
        match tag {
            b"PQTZ" if out.grid.is_none() => out.grid = beat_grid(section, head),
            b"PCOB" => cue_list(section, head, false, &mut out.basic),
            b"PCO2" => cue_list(section, head, true, &mut out.extended),
            _ => {}
        }
        at = end;
    }
    out
}

/// The first beat's time and tempo from a `PQTZ` section.
fn beat_grid(section: &[u8], head: usize) -> Option<(f64, f64)> {
    let count = be32(section, 20)? as usize;
    if count == 0 {
        return None;
    }
    let entry = head;
    // The entries must lie inside the section; only the first is read.
    entry.checked_add(8)?.le(&section.len()).then_some(())?;
    let tempo = be16(section, entry + 2)?;
    let ms = be32(section, entry + 4)?;
    (tempo > 0).then(|| (f64::from(ms) / 1000.0, f64::from(tempo) / 100.0))
}

/// The cues of a `PCOB` (`extended` false) or `PCO2` section.
fn cue_list(section: &[u8], head: usize, extended: bool, out: &mut Vec<RawCue>) {
    // `PCOB`: type u32, then 2 unknown bytes, then a u16 count. `PCO2`: type u32, u16 count.
    let Some(count) = be16(section, if extended { 16 } else { 18 }) else {
        return;
    };
    let mut at = head;
    for _ in 0..count {
        let (Some(tag), Some(entry_head), Some(entry_len)) = (
            tag_at(section, at),
            be32_at(section, at, 4),
            be32_at(section, at, 8),
        ) else {
            return;
        };
        let (entry_head, entry_len) = (entry_head as usize, entry_len as usize);
        let Some(end) = at.checked_add(entry_len).filter(|&e| e <= section.len()) else {
            return;
        };
        if entry_head < 12 || entry_len < entry_head {
            return;
        }
        let entry = &section[at..end];
        let parsed = match tag {
            b"PCPT" => basic_cue(entry),
            b"PCP2" => extended_cue(entry),
            _ => None,
        };
        out.extend(parsed);
        at = end;
    }
}

/// A `PCPT`: the hot cue number at 12, the kind at 28, the times at 32 and 36.
fn basic_cue(e: &[u8]) -> Option<RawCue> {
    Some(RawCue {
        hot_cue: be32(e, 12)?,
        is_loop: *e.get(28)? == 2,
        start_ms: be32(e, 32)?,
        end_ms: be32(e, 36)?,
        color: None,
        comment: None,
    })
}

/// A `PCP2`: as `PCPT`, then the loop size, the comment, and the hot cue's colour.
fn extended_cue(e: &[u8]) -> Option<RawCue> {
    let hot_cue = be32(e, 12)?;
    let is_loop = *e.get(16)? == 2;
    let start_ms = be32(e, 20)?;
    let end_ms = be32(e, 24)?;
    // Loop size to 40, then the comment's byte length and its UTF-16BE text.
    let len = be32(e, 40)? as usize;
    let text = e.get(44..44usize.checked_add(len)?)?;
    let units: Vec<u16> = text
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes(*p))
        .take_while(|&u| u != 0)
        .collect();
    let comment = String::from_utf16_lossy(&units);
    let after = 44usize.checked_add(len)?;
    let code = *e.get(after)?;
    let rgb = e.get(after.checked_add(1)?..after.checked_add(4)?)?;
    let color = palette(code).or_else(|| {
        (rgb != [0, 0, 0])
            .then(|| u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]))
    });
    Some(RawCue {
        hot_cue,
        is_loop,
        start_ms,
        end_ms,
        color,
        comment: (!comment.is_empty()).then_some(comment),
    })
}

/// rekordbox's hot cue colour for a colour code (the colours the app shows). The table is the
/// public colour list of the Deep Symmetry analysis of rekordbox exports,
/// <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>.
fn palette(code: u8) -> Option<u32> {
    const COLORS: [u32; 62] = [
        0x305aff, 0x5073ff, 0x508cff, 0x50a0ff, 0x50b4ff, 0x50b0f2, 0x50aee8, 0x45acdb, 0x00e0ff,
        0x19daf0, 0x32d2e6, 0x21b4b9, 0x20aaa0, 0x1fa392, 0x19a08c, 0x14a584, 0x14aa7d, 0x10b176,
        0x30d26e, 0x37de5a, 0x3ceb50, 0x28e214, 0x7dc13d, 0x8cc832, 0x9bd723, 0xa5e116, 0xa5dc0a,
        0xaad208, 0xb4c805, 0xb4be04, 0xbab404, 0xc3af04, 0xe1aa00, 0xffa000, 0xff9600, 0xff8c00,
        0xff7500, 0xe0641b, 0xe0461e, 0xe0301e, 0xe02823, 0xe62828, 0xff376f, 0xff2d6f, 0xff127b,
        0xf51e8c, 0xeb2da0, 0xe637b4, 0xde44cf, 0xde448d, 0xe630b4, 0xe619dc, 0xe600ff, 0xdc00ff,
        0xcc00ff, 0xb432ff, 0xb93cff, 0xc542ff, 0xaa5aff, 0xaa72ff, 0x8272ff, 0x6473ff,
    ];
    usize::from(code)
        .checked_sub(1)
        .and_then(|i| COLORS.get(i))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(tag: &[u8; 4], head: usize, body: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend((head as u32).to_be_bytes());
        out.extend(((head + body.len()) as u32).to_be_bytes());
        out.resize(head, 0);
        out.extend(body);
        out
    }

    fn file(sections: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = sections.concat();
        let mut out = b"PMAI".to_vec();
        out.extend(28u32.to_be_bytes());
        out.extend((28 + body.len() as u32).to_be_bytes());
        out.resize(28, 0);
        out.extend(body);
        out
    }

    fn beat_grid() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(1u16.to_be_bytes());
        body.extend(12600u16.to_be_bytes());
        body.extend(250u32.to_be_bytes());
        // The header carries the two unknown words and the beat count: 24 bytes in all.
        let mut full = section(b"PQTZ", 24, &body);
        full[20..24].copy_from_slice(&1u32.to_be_bytes());
        full
    }

    fn hot_cue_list() -> Vec<u8> {
        let mut entry = vec![0u8; 56];
        entry[..4].copy_from_slice(b"PCPT");
        entry[4..8].copy_from_slice(&28u32.to_be_bytes());
        entry[8..12].copy_from_slice(&56u32.to_be_bytes());
        entry[12..16].copy_from_slice(&2u32.to_be_bytes());
        entry[28] = 1;
        entry[32..36].copy_from_slice(&5216u32.to_be_bytes());
        let mut list = section(b"PCOB", 24, &entry);
        list[12..16].copy_from_slice(&1u32.to_be_bytes());
        list[18..20].copy_from_slice(&1u16.to_be_bytes());
        list
    }

    #[test]
    fn an_anlz_with_an_unknown_tag_still_reads() {
        let plain = file(&[beat_grid(), hot_cue_list()]);
        let with_unknown = file(&[
            beat_grid(),
            section(b"PXYZ", 16, &[1, 2, 3, 4, 5, 6, 7, 8]),
            hot_cue_list(),
            section(b"PXYZ", 12, &[]),
        ]);
        let expected = combine(&plain, None);
        assert_eq!(expected.grid, Some((0.25, 126.0)));
        assert_eq!(expected.cues.len(), 1);
        assert_eq!(expected.cues[0].slot, 1);
        assert!((expected.cues[0].seconds - 5.216).abs() < 1e-9);
        assert_eq!(combine(&with_unknown, None), expected);
    }

    #[test]
    fn a_damaged_file_never_panics() {
        let good = file(&[beat_grid(), hot_cue_list()]);
        for cut in 0..good.len() {
            let _ = combine(&good[..cut], Some(&good[cut..]));
        }
        let mut huge = good.clone();
        huge[32..36].copy_from_slice(&u32::MAX.to_be_bytes());
        let _ = combine(&huge, None);
    }
}
