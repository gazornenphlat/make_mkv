//! Reads the programs (titles) of a DVD title set from its IFO menu file, VTS_NN_0.IFO.
//! Layout follows the DVD-Video format: a program chain (PGC) lists cells, and each cell is a range of sectors.

use crate::logic::{be_u16, be_u32};

/// Sectors are 2048 bytes on DVD.
const SECTOR: usize = 2048;

/// One program chain: a playable title with its length and the sector ranges it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub duration_secs: f64,
    /// Sector ranges `[first, end)` in playback order, relative to the title set's VOB files.
    pub cells: Vec<(u32, u32)>,
}

/// Programs of one title set. None when the file is not a title set menu or is cut short.
pub fn parse_vts_programs(ifo: &[u8]) -> Option<Vec<Program>> {
    if !ifo.starts_with(b"DVDVIDEO-VTS") {
        return None;
    }
    // VTS_PGCI: sector of the program chain table, at byte 0xCC of the file.
    let pgci = be_u32(ifo, 0xCC)? as usize * SECTOR;
    // The first language unit's table is at +12, as an offset from the start of VTS_PGCI.
    let lu_start = be_u32(ifo, pgci + 8 + 4)? as usize;
    let pgcit = pgci + lu_start;
    let count = be_u16(ifo, pgcit)? as usize;
    let mut programs = Vec::with_capacity(count);
    for i in 0..count {
        // Each entry is 8 bytes: category (4) and the chain's offset from the table start (4).
        let start = be_u32(ifo, pgcit + 8 + i * 8 + 4)? as usize;
        let pgc = pgcit + start;
        let cell_count = *ifo.get(pgc + 3)? as usize;
        let cell_table = pgc + be_u16(ifo, pgc + 0xE8)? as usize;
        let mut cells = Vec::with_capacity(cell_count);
        for c in 0..cell_count {
            let entry = cell_table + c * 24;
            let first = be_u32(ifo, entry + 8)?;
            let last = be_u32(ifo, entry + 20)?;
            cells.push((first, last.checked_add(1)?));
        }
        programs.push(Program {
            duration_secs: bcd_time(ifo.get(pgc + 4..pgc + 8)?)?,
            cells,
        });
    }
    Some(programs)
}

/// Playback time as four BCD bytes: hours, minutes, seconds and frames (with the frame rate in the top bits).
fn bcd_time(bytes: &[u8]) -> Option<f64> {
    let bcd = |v: u8| -> Option<f64> {
        let (hi, lo) = (v >> 4, v & 0x0F);
        (hi <= 9 && lo <= 9).then_some(f64::from(hi * 10 + lo))
    };
    let hours = bcd(*bytes.first()?)?;
    let minutes = bcd(*bytes.get(1)?)?;
    let seconds = bcd(*bytes.get(2)?)?;
    let last = *bytes.get(3)?;
    let frames = bcd(last & 0x3F)?;
    let fps = if last >> 6 == 1 { 25.0 } else { 29.97 };
    Some(hours * 3600.0 + minutes * 60.0 + seconds + frames / fps)
}

/// Builds a minimal title set menu with one program, for tests.
#[cfg(test)]
pub fn synthetic_ifo(time: [u8; 4], cells: &[(u32, u32)]) -> Vec<u8> {
    let mut b = vec![0u8; 2080 + 256 + 24 * cells.len()];
    b[..12].copy_from_slice(b"DVDVIDEO-VTS");
    b[0xCC..0xD0].copy_from_slice(&1u32.to_be_bytes());
    let pgci = 2048;
    b[pgci..pgci + 2].copy_from_slice(&1u16.to_be_bytes());
    b[pgci + 12..pgci + 16].copy_from_slice(&16u32.to_be_bytes());
    let pgcit = pgci + 16;
    b[pgcit..pgcit + 2].copy_from_slice(&1u16.to_be_bytes());
    b[pgcit + 12..pgcit + 16].copy_from_slice(&16u32.to_be_bytes());
    let pgc = pgcit + 16;
    b[pgc + 3] = cells.len() as u8;
    b[pgc + 4..pgc + 8].copy_from_slice(&time);
    b[pgc + 0xE8..pgc + 0xEA].copy_from_slice(&256u16.to_be_bytes());
    for (i, (first, end)) in cells.iter().enumerate() {
        let e = pgc + 256 + i * 24;
        b[e + 8..e + 12].copy_from_slice(&first.to_be_bytes());
        b[e + 20..e + 24].copy_from_slice(&(end - 1).to_be_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_time_and_cells_are_read() {
        // 00:01:30 plus 25 frames at 25 fps (frame-rate bits 01) is 91 seconds.
        let ifo = synthetic_ifo([0x00, 0x01, 0x30, 0x65], &[(0, 100), (100, 200)]);
        let programs = parse_vts_programs(&ifo).unwrap();
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].duration_secs, 91.0);
        assert_eq!(programs[0].cells, vec![(0, 100), (100, 200)]);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(parse_vts_programs(b"VIDEO_TS nope").is_none());
        assert!(parse_vts_programs(&synthetic_ifo([0; 4], &[])[..0x100]).is_none());
    }
}
