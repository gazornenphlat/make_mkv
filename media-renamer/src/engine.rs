//! The rules from MECHANICS.md, written by us: name parsing, the naming
//! format, matching, and the rename actions. No UI code here.

use fancy_regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const VIDEO_EXT: &[&str] = &[
    "mkv", "mk3d", "mp4", "m4v", "3gp", "iso", "mov", "avi", "mpg", "mpeg", "vob", "ts", "tp",
    "m2ts", "m4s", "ogg", "ogm", "divx", "asf", "wmv", "wtv", "dvr-ms", "rec", "webm", "flv", "rm",
    "rmvb", "rmp4", "tivo", "nuv", "3dsbs", "3dtab", "strm", "strmlnk", "streamlnk",
];
pub const SUBTITLE_EXT: &[&str] = &[
    "srt", "sub", "ssa", "ass", "smi", "sami", "mpl", "sup", "vtt", "ttml", "vobsub", "idx",
];

/// Largest number of characters kept in a name before the extension (MECHANICS.md).
const NAME_LIMIT: usize = 247;
/// Highest number tried for "name (n).ext" when a name is taken.
const AUTO_NUMBER_MAX: u32 = 99;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Episode,
    Movie,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchMode {
    Opportunistic,
    Strict,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Move,
    Copy,
    HardLink,
    Symlink,
    Test,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conflict {
    Skip,
    Replace,
    AutoNumber,
}

pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub title: String,
}

/// Reads lines such as "S01E02 Title" or "1x02 Title". Other lines are ignored.
pub fn parse_episode_list(text: &str) -> Vec<Episode> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^\s*[Ss]?(\d{1,3})\s*[xXeE]\s*(\d{1,4})\b[\s:.|-]*(.*?)\s*$").unwrap());
    text.lines()
        .filter_map(|line| {
            let c = re.captures(line).ok().flatten()?;
            Some(Episode {
                season: c[1].parse().ok()?,
                number: c[2].parse().ok()?,
                title: c[3].trim().to_string(),
            })
        })
        .collect()
}

// Season and episode patterns, in the order of SeasonEpisodeMatcher. Java's
// \p{Alnum} is written as [[:alnum:]] here, \p{Digit} as [0-9].
const PATTERNS: [&str; 17] = [
    r"(?<![[:alnum:]])(?i:Season|Series)[^[:alnum:]]{0,3}(\d{1,4})[^[:alnum:]]{0,3}(?i:Episode|Part|Act)[^[:alnum:]]{0,3}((?:\d{1,4}(?:\D|$))+)[^[:alnum:]]{0,3}(?![0-9])",
    r"(?<![[:alnum:]]|[-])[Ss](\d{1,4})[Ee](\d{2,3})[-][Ee]?(\d{2,4})(?![[:alnum:]]|[-])",
    r"(?<![0-9])[Ss](\d{1,4})[^[:alnum:]]{0,3}(?i:ep|e|p|-)((?:(?<=[^._ ])[Ee]?[Pp]?\d{1,4}(?:\D|$))+)",
    r"(?<![[:alnum:]])(\d{1,2}x\d{2}(?:[-._ ]\d{1,2}x\d{2})+)(?![0-9])",
    r"(?<![[:alnum:]])[Ss]?(\d{1,4})[xXEe]((?:(?<=[^._ ])\d{2,4}(?:\D|$))+)",
    r"(?<![0-9])(?:S|s|Season|Series)[._ ]?(\d{1,4})(?:[._ ]-[._ ])(\d{1,4}(?:[-]\d{1,4})*)(?:\D|$)",
    r"(?<![[:alnum:]]|\d{4}[.])(\d{1,2})[.]((?:(?<=[^._ ])\d{2}(?:\D|$))+)",
    r"(?<=[[:alnum:]])[^[:alnum:]]{0,3}(?i:EP|Episode|Act)[^[:alnum:]]{0,3}(\d{2,4})(?![-]?[0-9])",
    r"(?<![[:alnum:]]|[-])(\d{2,3})[-](\d{2,3})(?![[:alnum:]]|[-])",
    r"(?<![[:alnum:]])SP(?:[._ ]-[._ ])?(\d{1,2}(?:[-]\d{1,2})*)(?:\D|$)",
    // 10 is a union of 14, 15 and 16; the union is handled in detect().
    r"",
    r"(?<=[._ ][-][._ ])(\d{1,4})(?![[:alnum:]])",
    r"[Ss](\d{2})[Ee](\d{2})",
    r"(?<![0-9])(\d{1})(\d{2})(?![[:alnum:]])(?:.*)",
    r"(?<![[:alnum:]])(\d{2}|\d{4})?[^[:alnum:]]{0,3}((?:(?i:e|p|ep|episode|act)[^[:alnum:]]{0,3}\d{1,4})+)(?![0-9])",
    r"(?<![[:alnum:]])([0-2]?\d?)(\d{2})(\d{2})?(?![[:alnum:]])",
    r"(?<![[:alnum:]])(\d{1,2})[^._ ]?(?i:of)[^._ ]?(\d{1,2})(?![0-9])",
];

fn compiled() -> &'static Vec<Option<Regex>> {
    static RES: OnceLock<Vec<Option<Regex>>> = OnceLock::new();
    RES.get_or_init(|| {
        PATTERNS
            .iter()
            .map(|p| if p.is_empty() { None } else { Some(Regex::new(p).unwrap()) })
            .collect()
    })
}

/// Season and episode numbers found in a file name.
#[derive(Debug, PartialEq)]
pub struct Detected {
    pub season: Option<u32>,
    pub episodes: Vec<u32>,
}

fn num(s: &str) -> Option<u32> {
    s.parse().ok()
}

fn numbers(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit()).filter_map(num).collect()
}

/// Tries the patterns in order (Strict: the first 7 only) and applies the sanity limits.
pub fn detect(name: &str, mode: MatchMode) -> Option<Detected> {
    let res = compiled();
    // Strict uses patterns 0 to 6; Opportunistic uses all, with the union in place of 10.
    let order: Vec<usize> = match mode {
        MatchMode::Strict => (0..=6).collect(),
        MatchMode::Opportunistic => vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
    };
    let (season_max, episode_max, abs_max) = match mode {
        MatchMode::Strict => (10, 30, u32::MAX),
        MatchMode::Opportunistic => (30, 50, 1000),
    };
    for idx in order {
        let candidates: Vec<usize> = if idx == 10 { vec![14, 15, 16] } else { vec![idx] };
        for ci in candidates {
            let Some(re) = &res[ci] else { continue };
            let Some(c) = re.captures(name).ok().flatten() else { continue };
            let d = match ci {
                0 | 2 | 4 | 5 | 6 | 12 | 13 => Detected {
                    season: num(&c[1]),
                    episodes: numbers(c.get(2).map_or("", |m| m.as_str())),
                },
                1 => Detected {
                    season: num(&c[1]),
                    episodes: match (num(&c[2]), num(&c[3])) {
                        (Some(a), Some(b)) if b >= a => (a..=b).collect(),
                        _ => vec![],
                    },
                },
                3 => {
                    // "1x02-1x03": the numbers alternate season, episode, season, episode...
                    let nums: Vec<u32> = c[1].split(|ch: char| !ch.is_ascii_digit()).filter_map(num).collect();
                    Detected {
                        season: nums.first().copied(),
                        episodes: nums.iter().skip(1).step_by(2).copied().collect(),
                    }
                }
                7 | 8 | 9 | 11 => Detected {
                    season: if ci == 9 { Some(0) } else { None },
                    episodes: if ci == 8 {
                        match (num(&c[1]), num(&c[2])) {
                            (Some(a), Some(b)) if b >= a => (a..=b).collect(),
                            _ => vec![],
                        }
                    } else {
                        numbers(&c[1])
                    },
                },
                10 => unreachable!(),
                14 => Detected { season: None, episodes: numbers(&c[2]) },
                15 => Detected { season: num(&c[1]).filter(|s| *s > 0), episodes: num(&c[2]).into_iter().collect() },
                16 => Detected { season: None, episodes: num(&c[1]).into_iter().collect() },
                _ => continue,
            };
            // Sanity limits from the sheet.
            if d.season.is_some_and(|s| s > season_max) {
                continue;
            }
            if d.episodes.iter().any(|&e| e > episode_max && d.season.is_some()) {
                continue;
            }
            if d.season.is_none() && d.episodes.iter().any(|&e| e > abs_max) {
                continue;
            }
            if d.episodes.is_empty() {
                continue;
            }
            return Some(d);
        }
    }
    None
}

/// Movie title and year from a file name.
pub struct MovieName {
    pub title: String,
    pub year: Option<u32>,
}

/// Quality tags that end the title (estimated: the program's own list was not read).
const QUALITY_TAG: &str = r"(?i)[\s(\[-]+(\d{3,4}p|bluray|brrip|webrip|web[ -]?dl|hdtv|dvdrip|x26[45]|h\.?26[45]|hevc)\b";
/// Years a movie name may carry: DateMatcher.DEFAULT_SANITY (1930 to 2040) filters the found years.
const YEAR_RANGE_START: u32 = 1930;
const YEAR_RANGE_END: u32 = 2040;

pub fn parse_movie(stem: &str) -> MovieName {
    static YEAR: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let spaced: String = stem.replace(['.', '_'], " ");
    // The program's YEAR pattern: a whole-word 19xx or 20xx, then the year window is checked.
    let year_re = YEAR.get_or_init(|| Regex::new(r"\b(?:19|20)[0-9]{2}\b").unwrap());
    let tag_re = TAG.get_or_init(|| Regex::new(QUALITY_TAG).unwrap());
    let cut = tag_re.find(&spaced).ok().flatten().map(|m| m.start()).unwrap_or(spaced.len());
    let head = &spaced[..cut];
    let year_match = year_re
        .find_iter(head)
        .filter_map(|m| m.ok())
        .filter(|m| num(m.as_str()).is_some_and(|y| (YEAR_RANGE_START..=YEAR_RANGE_END).contains(&y)))
        .last();
    let (title_part, year) = match year_match {
        Some(m) => (&head[..m.start()], num(m.as_str())),
        None => (head, None),
    };
    let title = title_part.trim().trim_end_matches(['(', '[', '-', ' ']).trim().to_string();
    let title = if title.is_empty() { head.trim().to_string() } else { title };
    MovieName { title, year }
}

/// Removes characters that are not allowed in file names, as FileUtilities.validateFileName does.
pub fn validate_name(s: &str) -> String {
    let mut out: String = s
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
        .collect();
    loop {
        let trimmed = out.trim().trim_start_matches('.').trim_end_matches('.').to_string();
        let trimmed = if trimmed.chars().count() > NAME_LIMIT {
            trimmed.chars().take(NAME_LIMIT).collect()
        } else {
            trimmed
        };
        if trimmed == out {
            return out;
        }
        out = trimmed;
    }
}

/// Values a naming format can use.
pub struct Ctx {
    pub name: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub title: String,
    pub year: Option<u32>,
    pub subtitle: String,
}

fn base_value(id: &str, c: &Ctx) -> String {
    match id {
        "n" => c.name.clone(),
        "s" => c.season.map(|v| v.to_string()).unwrap_or_default(),
        "e" => c.episode.map(|v| v.to_string()).unwrap_or_default(),
        "t" => c.title.clone(),
        "y" => c.year.map(|v| v.to_string()).unwrap_or_default(),
        "s00e00" => match (c.season, c.episode) {
            (Some(s), Some(e)) => format!("S{s:02}E{e:02}"),
            (None, Some(e)) => format!("E{e:02}"),
            _ => String::new(),
        },
        "sxe" => match (c.season, c.episode) {
            (Some(s), Some(e)) => format!("{s}x{e:02}"),
            (None, Some(e)) => format!("{e:02}"),
            _ => String::new(),
        },
        "subt" => c.subtitle.clone(),
        _ => String::new(),
    }
}

/// Parses an argument such as ' - ' (quoted) or 2 (number).
fn arg_text(arg: &str) -> String {
    let a = arg.trim();
    if a.len() >= 2 && a.starts_with('\'') && a.ends_with('\'') {
        a[1..a.len() - 1].to_string()
    } else {
        a.to_string()
    }
}

/// Splits `n.colon(' - ')` into parts at dots that are outside quotes and brackets.
fn split_calls(expr: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut quoted, mut depth, mut start) = (false, 0i32, 0usize);
    for (i, ch) in expr.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            '.' if !quoted && depth == 0 => {
                parts.push(&expr[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&expr[start..]);
    parts
}

/// Applies one `.method(args)` step to a value.
fn apply_method(value: String, call: &str) -> String {
    let (name, args) = match call.find('(') {
        Some(i) => (&call[..i], call[i + 1..].trim_end_matches(')')),
        None => (call, ""),
    };
    static COLON: OnceLock<Regex> = OnceLock::new();
    static SPACE_SEP: OnceLock<Regex> = OnceLock::new();
    static SPACE_RUN: OnceLock<Regex> = OnceLock::new();
    static DIGITS: OnceLock<Regex> = OnceLock::new();
    match name {
        // RegularExpressions.COLON `\s*[:꞉]+\s*` becomes the separator (ExpressionFormatMethods.colon).
        "colon" => COLON
            .get_or_init(|| Regex::new(r"\s*[:꞉]+\s*").unwrap())
            .replace_all(&value, arg_text(args).as_str())
            .into_owned(),
        // Normalization.normalizeSpace: `[:?._]` to a space and trim, then `\s+` to the separator and trim.
        "space" => {
            let a = SPACE_SEP.get_or_init(|| Regex::new(r"[:?._]").unwrap()).replace_all(&value, " ");
            let a = a.trim();
            SPACE_RUN
                .get_or_init(|| Regex::new(r"\s+").unwrap())
                .replace_all(a, arg_text(args).as_str())
                .trim()
                .to_string()
        }
        // ExpressionFormatMethods.pad: every digit run is zero-padded to the width.
        "pad" => {
            let width: usize = args.trim().parse().unwrap_or(0);
            let re = DIGITS.get_or_init(|| Regex::new(r"\d+").unwrap());
            let mut out = String::new();
            let mut last = 0;
            for m in re.find_iter(&value).flatten() {
                out.push_str(&value[last..m.start()]);
                out.push_str(&format!("{:0>width$}", m.as_str(), width = width));
                last = m.end();
            }
            out.push_str(&value[last..]);
            out
        }
        // ExpressionFormatMethods.clean = validateFileName(stripReleaseInfo(s)).
        // shortcut: release tags (resolution, codec, source) are not stripped, port ReleaseInfo.cleanRelease to upgrade.
        "clean" => validate_name(&value),
        _ => value,
    }
}

/// Renders a naming format such as `{n} - {s00e00} - {t}` with the given values.
pub fn render(format: &str, c: &Ctx) -> String {
    let mut out = String::new();
    let mut rest = format;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let expr = &after[..close];
        let mut parts = split_calls(expr).into_iter();
        let head = parts.next().unwrap_or("");
        let mut value = if head.starts_with('\'') {
            arg_text(head)
        } else {
            base_value(head, c)
        };
        for call in parts {
            value = apply_method(value, call);
        }
        out.push_str(&value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// One file in the preview.
pub struct PlanItem {
    pub src: PathBuf,
    pub detected: String,
    pub title: String,
    /// New path (without extension change), or None when nothing matched.
    pub dst: Option<PathBuf>,
    pub status: String,
}

fn stem_and_ext(p: &Path) -> (String, String) {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    (stem, ext)
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Input for building the preview.
pub struct Settings<'a> {
    pub kind: Kind,
    pub mode: MatchMode,
    pub format: &'a str,
    pub series: &'a str,
    pub episodes: &'a [Episode],
}

/// Builds the preview: what each video file would become.
pub fn plan(files: &[(PathBuf, String)], s: &Settings) -> Vec<PlanItem> {
    let mut items = Vec::new();
    for (path, title_override) in files {
        let ext = ext_of(path);
        if !VIDEO_EXT.contains(&ext.as_str()) {
            continue;
        }
        let (stem, _) = stem_and_ext(path);
        let item = match s.kind {
            Kind::Episode => plan_episode(path, &stem, title_override, s),
            Kind::Movie => plan_movie(path, &stem, title_override, s),
        };
        items.push(item);
    }
    // A subtitle next to a renamed video follows it and keeps its language part (".en").
    for (sub, _) in files.iter().filter(|(p, _)| SUBTITLE_EXT.contains(&ext_of(p).as_str())) {
        let (sub_stem, sub_ext) = stem_and_ext(sub);
        let follow = items.iter().find_map(|v| {
            let (video_stem, _) = stem_and_ext(&v.src);
            let dst = v.dst.as_ref()?;
            let rest = sub_stem.strip_prefix(video_stem.as_str())?;
            let dst_stem = stem_and_ext(dst).0;
            let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
            Some(parent.join(format!("{dst_stem}{rest}.{sub_ext}")))
        });
        items.push(PlanItem {
            src: sub.clone(),
            detected: "subtitle".into(),
            title: String::new(),
            dst: follow.clone(),
            status: if follow.is_some() { "ready (follows its video)".into() } else { "no renamed video to follow".into() },
        });
    }
    // Two files that would get the same name are flagged, not renamed.
    let mut seen: Vec<PathBuf> = Vec::new();
    for item in items.iter_mut() {
        if let Some(d) = &item.dst {
            if seen.contains(d) {
                item.dst = None;
                item.status = "same name as another file".into();
            } else {
                seen.push(d.clone());
            }
        }
    }
    items
}

fn plan_episode(path: &Path, stem: &str, title_override: &str, s: &Settings) -> PlanItem {
    let Some(det) = detect(stem, s.mode) else {
        return no_match(path, "no season or episode number found");
    };
    let episode = det.episodes[0];
    let season = det.season;
    let found = s
        .episodes
        .iter()
        .find(|e| season.is_some_and(|sn| sn == e.season) && e.number == episode)
        .or_else(|| {
            // Opportunistic also takes a match on the episode number alone.
            if s.mode == MatchMode::Opportunistic {
                s.episodes.iter().find(|e| e.number == episode)
            } else {
                None
            }
        });
    let Some(entry) = found else {
        return no_match(path, "no episode in the list");
    };
    let title = if title_override.trim().is_empty() {
        entry.title.clone()
    } else {
        title_override.trim().to_string()
    };
    let ctx = Ctx {
        name: s.series.trim().to_string(),
        season: Some(entry.season),
        episode: Some(entry.number),
        title: title.clone(),
        year: None,
        subtitle: String::new(),
    };
    build(path, &render(s.format, &ctx), format!("S{:02}E{:02}", entry.season, entry.number), title)
}

fn plan_movie(path: &Path, stem: &str, title_override: &str, s: &Settings) -> PlanItem {
    let parsed = parse_movie(stem);
    if s.mode == MatchMode::Strict && parsed.year.is_none() {
        return no_match(path, "strict: no year in the name");
    }
    let name = if title_override.trim().is_empty() {
        parsed.title.clone()
    } else {
        title_override.trim().to_string()
    };
    let detected = parsed.year.map(|y| y.to_string()).unwrap_or_default();
    let ctx = Ctx {
        name: name.clone(),
        season: None,
        episode: None,
        title: String::new(),
        year: parsed.year,
        subtitle: String::new(),
    };
    build(path, &render(s.format, &ctx), detected, name)
}

fn no_match(path: &Path, why: &str) -> PlanItem {
    PlanItem {
        src: path.to_path_buf(),
        detected: String::new(),
        title: String::new(),
        dst: None,
        status: why.into(),
    }
}

fn build(path: &Path, rendered: &str, detected: String, title: String) -> PlanItem {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    // "/" in a format makes folders; each part is cleaned on its own.
    let parts: Vec<String> = rendered.split('/').map(validate_name).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return no_match(path, "the new name is empty");
    }
    let (_, ext) = stem_and_ext(path);
    let mut dst = parent;
    for p in &parts[..parts.len() - 1] {
        dst.push(p);
    }
    let last = &parts[parts.len() - 1];
    let file = if ext.is_empty() { last.clone() } else { format!("{last}.{ext}") };
    dst.push(file);
    let status = if dst == path {
        "already named this way".to_string()
    } else {
        "ready".to_string()
    };
    PlanItem { src: path.to_path_buf(), detected, title, dst: Some(dst), status }
}

/// One finished operation, kept for undo.
#[derive(Clone)]
pub struct Op {
    pub action: Action,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub replaced: bool,
}

/// Picks the destination after the conflict rule. None means skip.
fn resolve_conflict(dst: &Path, conflict: Conflict) -> Result<(PathBuf, bool), String> {
    if !dst.exists() {
        return Ok((dst.to_path_buf(), false));
    }
    match conflict {
        Conflict::Skip => Err("skipped: the name already exists".into()),
        Conflict::Replace => Ok((dst.to_path_buf(), true)),
        Conflict::AutoNumber => {
            // StandardConflictAction.AUTO: the first free "name (n).ext" with n from 1 to AUTO_NUMBER_MAX.
            let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
            let (stem, ext) = stem_and_ext(dst);
            for n in 1..=AUTO_NUMBER_MAX {
                let candidate = parent.join(if ext.is_empty() {
                    format!("{stem} ({n})")
                } else {
                    format!("{stem} ({n}).{ext}")
                });
                if !candidate.exists() {
                    return Ok((candidate, false));
                }
            }
            Err("no free number from 1 to 99".into())
        }
    }
}

/// Carries out the plan. Returns one line per file and the operations for undo.
pub fn execute(plan: &[PlanItem], action: Action, conflict: Conflict) -> (Vec<String>, Vec<Op>) {
    let mut log = Vec::new();
    let mut ops = Vec::new();
    for item in plan {
        let Some(dst0) = &item.dst else {
            log.push(format!("{}: {}", file_name(&item.src), item.status));
            continue;
        };
        if action == Action::Test {
            log.push(format!("{} -> {}", file_name(&item.src), file_name(dst0)));
            continue;
        }
        let (dst, replaced) = match resolve_conflict(dst0, conflict) {
            Ok(v) => v,
            Err(msg) => {
                log.push(format!("{}: {msg}", file_name(&item.src)));
                continue;
            }
        };
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log.push(format!("{}: could not make folder: {e}", file_name(&item.src)));
                continue;
            }
        }
        match perform(action, &item.src, &dst) {
            Ok(()) => {
                log.push(format!("{} -> {}", file_name(&item.src), file_name(&dst)));
                ops.push(Op { action, src: item.src.clone(), dst, replaced });
            }
            Err(e) => log.push(format!("{}: {e}", file_name(&item.src))),
        }
    }
    (log, ops)
}

fn perform(action: Action, src: &Path, dst: &Path) -> Result<(), String> {
    let r = match action {
        Action::Move => std::fs::rename(src, dst).or_else(|_| {
            // Across drives rename fails: copy, then remove the original.
            std::fs::copy(src, dst).and_then(|_| std::fs::remove_file(src))
        }),
        Action::Copy => std::fs::copy(src, dst).map(|_| ()),
        Action::HardLink => std::fs::hard_link(src, dst),
        #[cfg(windows)]
        Action::Symlink => std::os::windows::fs::symlink_file(src, dst),
        #[cfg(not(windows))]
        Action::Symlink => Err(std::io::Error::other("symbolic links are not supported on this system")),
        Action::Test => Ok(()),
    };
    r.map_err(|e| e.to_string())
}

/// Reverts operations in reverse order. Replaced files cannot come back.
pub fn undo(ops: &[Op]) -> Vec<String> {
    let mut log = Vec::new();
    for op in ops.iter().rev() {
        if op.replaced {
            log.push(format!("{}: cannot undo, the old file was replaced", file_name(&op.dst)));
            continue;
        }
        let r = match op.action {
            Action::Move => std::fs::rename(&op.dst, &op.src),
            _ => std::fs::remove_file(&op.dst),
        };
        match r {
            Ok(()) => log.push(format!("undone: {}", file_name(&op.dst))),
            Err(e) => log.push(format!("{}: {e}", file_name(&op.dst))),
        }
    }
    log
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_EP: &str = "{n} - {s00e00} - {t}{subt}";

    fn ctx() -> Ctx {
        Ctx {
            name: "Firefly".into(),
            season: Some(1),
            episode: Some(2),
            title: "Serenity".into(),
            year: None,
            subtitle: String::new(),
        }
    }

    #[test]
    fn detects_season_and_episode() {
        assert_eq!(detect("Firefly.S01E02.720p", MatchMode::Opportunistic).unwrap().season, Some(1));
        let d = detect("Firefly 1x02 Serenity", MatchMode::Opportunistic).unwrap();
        assert_eq!((d.season, d.episodes.clone()), (Some(1), vec![2]));
        assert!(detect("random file", MatchMode::Strict).is_none());
    }

    #[test]
    fn parses_episode_list_lines() {
        let eps = parse_episode_list("S01E02 Serenity\nnot an episode\n1x03 Bushwhacked");
        assert_eq!(eps.len(), 2);
        assert_eq!((eps[1].season, eps[1].number, eps[1].title.as_str()), (1, 3, "Bushwhacked"));
    }

    #[test]
    fn renders_default_episode_format() {
        assert_eq!(render("{n} - {s00e00} - {t}{subt}", &ctx()), "Firefly - S01E02 - Serenity");
    }

    #[test]
    fn renders_movie_format_with_colon_and_year() {
        let c = Ctx { name: "Avatar: The Way".into(), season: None, episode: None, title: String::new(), year: Some(2009), subtitle: String::new() };
        assert_eq!(render("{n.colon(' - ')} ({y}){subt}", &c), "Avatar - The Way (2009)");
    }

    #[test]
    fn renders_space_and_pad_like_the_program() {
        let c = Ctx { name: "a.b_c".into(), season: Some(1), episode: Some(7), title: String::new(), year: None, subtitle: String::new() };
        assert_eq!(render("{n.space('-')}", &c), "a-b-c");
        assert_eq!(render("{e.pad(3)} {sxe}", &c), "007 1x07");
    }

    #[test]
    fn cleans_illegal_characters() {
        assert_eq!(validate_name("a/b:c?d. "), "abcd");
    }

    #[test]
    fn plans_episode_with_its_subtitle() {
        let dir = Path::new("shows");
        let files = vec![
            (dir.join("Firefly.S01E02.720p.mkv"), String::new()),
            (dir.join("Firefly.S01E02.720p.en.srt"), String::new()),
            (dir.join("notes.txt"), String::new()),
        ];
        let eps = parse_episode_list("S01E02 Serenity");
        let s = Settings { kind: Kind::Episode, mode: MatchMode::Opportunistic, format: DEFAULT_EP, series: "Firefly", episodes: &eps };
        let plan = plan(&files, &s);
        assert_eq!(plan.len(), 2, "the txt file is not part of the plan");
        assert_eq!(plan[0].dst.as_deref(), Some(dir.join("Firefly - S01E02 - Serenity.mkv").as_path()));
        assert_eq!(plan[1].dst.as_deref(), Some(dir.join("Firefly - S01E02 - Serenity.en.srt").as_path()));
    }

    #[test]
    fn strict_episode_needs_the_season_too() {
        let files = vec![(Path::new("x").join("Firefly.E02.mkv"), String::new())];
        let eps = parse_episode_list("S01E02 Serenity");
        let strict = Settings { kind: Kind::Episode, mode: MatchMode::Strict, format: DEFAULT_EP, series: "Firefly", episodes: &eps };
        assert!(plan(&files, &strict)[0].dst.is_none());
        let loose = Settings { mode: MatchMode::Opportunistic, ..strict };
        assert!(plan(&files, &loose)[0].dst.is_some());
    }

    #[test]
    fn parses_movie_title_and_year() {
        let m = parse_movie("Avatar.2009.1080p.BluRay");
        assert_eq!((m.title.as_str(), m.year), ("Avatar", Some(2009)));
    }
}
