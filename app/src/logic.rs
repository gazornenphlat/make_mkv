//! Rules for finding, picking and naming titles. No window code here, so they can be tested.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Default minimum title length in seconds.
pub const DEFAULT_MIN_LENGTH_SECS: u32 = 120;
/// `%02u` becomes the title number and `%s` the disc or folder name.
pub const DEFAULT_NAME_TEMPLATE: &str = "title%02u.mkv";
/// Suffix used when the output name is already taken.
pub const CLASH_SUFFIX_TEMPLATE: &str = "_t%02u.mkv";

/// File types treated as titles. Only files you already have are read; protected discs are not decrypted.
const MEDIA_EXTENSIONS: [&str; 5] = ["mkv", "m2ts", "ts", "vob", "mp4"];
/// Blu-ray playlist times count in 45 kHz ticks.
const BD_TICKS_PER_SEC: f64 = 45_000.0;

#[derive(Clone, Debug)]
pub struct Title {
    /// 1-based position in the list, used in output names.
    pub number: usize,
    /// Name shown in the list.
    pub label: String,
    /// Files played in order. A disc title can be several files (DVD VOB parts, Blu-ray clips).
    pub sources: Vec<PathBuf>,
    pub size_bytes: u64,
    /// Length in seconds, when it is known.
    pub duration_secs: Option<f64>,
    pub selected: bool,
}

/// Fills in the title number and the disc name, e.g. `%s_%02u.mkv` gives `Movie_03.mkv`.
pub fn fill_name(template: &str, number: usize, disc: &str) -> String {
    template.replace("%02u", &format!("{number:02}")).replace("%s", &sanitize(disc))
}

/// Replaces characters that Windows does not allow in file names.
fn sanitize(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = clean.trim().trim_end_matches('.');
    if trimmed.is_empty() { "disc".to_string() } else { trimmed.to_string() }
}

/// The name a folder or disc is known by: its last folder name, or "disc" for a drive root.
pub fn folder_label(folder: &Path) -> String {
    folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "disc".to_string())
}

pub fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTENSIONS.iter().any(|m| e.eq_ignore_ascii_case(m)))
}

/// A title is too short only when its length is known and strictly below the minimum.
pub fn is_too_short(duration: Option<f64>, min_secs: u32) -> bool {
    duration.is_some_and(|d| d < f64::from(min_secs))
}

/// Drop a trailing `.mkv` (any case), so the clash suffix can replace it.
fn strip_mkv(name: &str) -> &str {
    match name.len().checked_sub(4).and_then(|i| name.get(i..)) {
        Some(tail) if tail.eq_ignore_ascii_case(".mkv") => &name[..name.len() - 4],
        _ => name,
    }
}

/// Output file for one title. Uses the clash name when the first choice already exists.
pub fn output_path(folder: &Path, template: &str, number: usize, disc: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let base = strip_mkv(&fill_name(template, number, disc)).to_string();
    let first = folder.join(format!("{base}.mkv"));
    if !exists(&first) {
        return first;
    }
    let clash = strip_mkv(&fill_name(CLASH_SUFFIX_TEMPLATE, number, disc)).to_string();
    folder.join(format!("{base}{clash}.mkv"))
}

/// What ffmpeg reads for a title: the file itself, or a `concat:` list of its parts.
pub fn input_arg(sources: &[PathBuf]) -> OsString {
    if let [one] = sources {
        return one.as_os_str().to_os_string();
    }
    let mut arg = OsString::from("concat:");
    for (i, path) in sources.iter().enumerate() {
        if i > 0 {
            arg.push("|");
        }
        arg.push(path.as_os_str());
    }
    arg
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiscKind {
    Empty,
    BluRay,
    Dvd,
    /// A disc is in the drive, but it has neither a BDMV nor a VIDEO_TS folder.
    Other,
}

/// Tells what is in a drive from the folders at its root. An empty drive has no readable root.
pub fn disc_kind(root: &Path) -> DiscKind {
    if std::fs::read_dir(root).is_err() {
        DiscKind::Empty
    } else if root.join("VIDEO_TS").is_dir() {
        DiscKind::Dvd
    } else if root.join("BDMV").is_dir() || root.join("BDAV").is_dir() {
        DiscKind::BluRay
    } else {
        DiscKind::Other
    }
}

/// One clip played by a Blu-ray playlist item.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayItem {
    /// Clip name without extension, e.g. `00001`. The file is STREAM/<clip>.m2ts.
    pub clip: String,
    pub in_time: u32,
    pub out_time: u32,
}

/// Reads the play items of a Blu-ray playlist (.mpls file). None when the data does not look right.
pub fn parse_mpls(bytes: &[u8]) -> Option<Vec<PlayItem>> {
    if !(bytes.starts_with(b"MPLS0100") || bytes.starts_with(b"MPLS0200")) {
        return None;
    }
    let start = be_u32(bytes, 8)? as usize;
    let count = be_u16(bytes, start + 6)? as usize;
    let mut pos = start + 10;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let len = be_u16(bytes, pos)? as usize;
        let body = bytes.get(pos + 2..pos + 2 + len)?;
        // Layout of one item body: clip name (5), codec (4), flags (2), STC id (1), in (4), out (4).
        let clip = std::str::from_utf8(body.get(0..5)?).ok()?.to_string();
        items.push(PlayItem { clip, in_time: be_u32(body, 12)?, out_time: be_u32(body, 16)? });
        pos += 2 + len;
    }
    Some(items)
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Blu-ray playlist ticks to seconds.
pub fn ticks_to_secs(ticks: u32) -> f64 {
    f64::from(ticks) / BD_TICKS_PER_SEC
}

/// DVD part file name `VTS_NN_P.VOB` gives (title set, part). `VIDEO_TS.VOB` gives None.
pub fn parse_vob_name(name: &str) -> Option<(u32, u32)> {
    let upper = name.to_ascii_uppercase();
    let rest = upper.strip_prefix("VTS_")?.strip_suffix(".VOB")?;
    let (set, part) = rest.split_once('_')?;
    Some((set.parse().ok()?, part.parse().ok()?))
}

/// ffprobe prints the length in seconds as plain text.
pub fn parse_duration(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0)
}

/// Share of one file finished, from an ffmpeg `-progress` line such as `out_time_us=5000000`.
pub fn progress_fraction(line: &str, duration_secs: Option<f64>) -> Option<f32> {
    let micros: f64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    let total = duration_secs?;
    Some((micros / 1_000_000.0 / total).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mpls_bytes(clips: &[(&str, u32, u32)]) -> Vec<u8> {
        let mut b = b"MPLS0200".to_vec();
        b.extend_from_slice(&20u32.to_be_bytes()); // PlayList starts at byte 20
        b.resize(20, 0);
        b.extend_from_slice(&0u32.to_be_bytes()); // length
        b.extend_from_slice(&0u16.to_be_bytes()); // reserved
        b.extend_from_slice(&(clips.len() as u16).to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes()); // sub paths
        for (clip, in_t, out_t) in clips {
            b.extend_from_slice(&20u16.to_be_bytes());
            b.extend_from_slice(clip.as_bytes());
            b.extend_from_slice(b"M2TS");
            b.extend_from_slice(&[0, 0, 0]); // flags and STC id
            b.extend_from_slice(&in_t.to_be_bytes());
            b.extend_from_slice(&out_t.to_be_bytes());
        }
        b
    }

    #[test]
    fn template_takes_number_and_disc_name() {
        assert_eq!(fill_name(DEFAULT_NAME_TEMPLATE, 3, "x"), "title03.mkv");
        assert_eq!(fill_name("%s_%02u.mkv", 4, "Movie: Disc"), "Movie_ Disc_04.mkv");
    }

    #[test]
    fn output_uses_clash_name_when_taken() {
        let dir = Path::new("out");
        assert_eq!(output_path(dir, DEFAULT_NAME_TEMPLATE, 3, "d", |_| false), dir.join("title03.mkv"));
        assert_eq!(output_path(dir, DEFAULT_NAME_TEMPLATE, 3, "d", |_| true), dir.join("title03_t03.mkv"));
    }

    #[test]
    fn mkv_suffix_is_removed_in_any_case() {
        assert_eq!(output_path(Path::new("o"), "Movie.MKV", 1, "d", |_| false), Path::new("o").join("Movie.mkv"));
    }

    #[test]
    fn short_titles_are_only_known_lengths_below_minimum() {
        assert!(is_too_short(Some(60.0), DEFAULT_MIN_LENGTH_SECS));
        assert!(!is_too_short(Some(120.0), DEFAULT_MIN_LENGTH_SECS));
        assert!(!is_too_short(None, DEFAULT_MIN_LENGTH_SECS));
    }

    #[test]
    fn disc_kind_reads_the_root_folders() {
        assert_eq!(disc_kind(Path::new("Z:\\no-such-drive\\")), DiscKind::Empty);
        let dir = std::env::temp_dir().join(format!("dr_disc_kind_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("BDMV")).unwrap();
        assert_eq!(disc_kind(&dir), DiscKind::BluRay);
        std::fs::remove_dir_all(dir.join("BDMV")).unwrap();
        std::fs::create_dir_all(dir.join("VIDEO_TS")).unwrap();
        assert_eq!(disc_kind(&dir), DiscKind::Dvd);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn duration_and_progress_parse() {
        assert_eq!(parse_duration(" 91.5\n"), Some(91.5));
        assert_eq!(parse_duration("N/A"), None);
        assert_eq!(progress_fraction("out_time_us=5000000", Some(10.0)), Some(0.5));
        assert_eq!(progress_fraction("frame=12", Some(10.0)), None);
    }

    #[test]
    fn playlist_items_are_read() {
        let items = parse_mpls(&mpls_bytes(&[("00001", 0, 45_000 * 90), ("00002", 45_000, 45_000 * 3)])).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].clip, "00001");
        assert_eq!(ticks_to_secs(items[0].out_time - items[0].in_time), 90.0);
        assert_eq!(items[1].clip, "00002");
        assert!(parse_mpls(b"NOTMPLS!").is_none());
    }

    #[test]
    fn vob_names_give_set_and_part() {
        assert_eq!(parse_vob_name("VTS_01_2.VOB"), Some((1, 2)));
        assert_eq!(parse_vob_name("vts_12_0.vob"), Some((12, 0)));
        assert_eq!(parse_vob_name("VIDEO_TS.VOB"), None);
    }

    #[test]
    fn input_is_file_or_concat_list() {
        assert_eq!(input_arg(&[PathBuf::from("a.vob")]), OsString::from("a.vob"));
        assert_eq!(
            input_arg(&[PathBuf::from("a.vob"), PathBuf::from("b.vob")]),
            OsString::from("concat:a.vob|b.vob")
        );
    }
}
