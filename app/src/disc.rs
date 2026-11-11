//! Finds the titles on a DVD or Blu-ray folder: Blu-ray playlists and DVD title sets.

use crate::{drives, logic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One title found on a disc, before it gets its number.
pub struct FoundTitle {
    pub name: String,
    /// Files played in order.
    pub sources: Vec<PathBuf>,
    /// Known from the disc's own index. DVD titles leave this empty and are measured with ffprobe.
    pub duration_secs: Option<f64>,
    pub size_bytes: u64,
}

/// Titles for a DVD or Blu-ray layout. None when the folder has neither, or has no usable titles.
pub fn find_disc_titles(folder: &Path) -> Option<Vec<FoundTitle>> {
    if let Some(bdmv) = blu_ray_root(folder) {
        if let Some(titles) = blu_ray_titles(&bdmv, disc_name(folder).as_deref()) {
            return Some(titles);
        }
    }
    let video_ts = dvd_root(folder)?;
    let titles = dvd_titles(&video_ts, disc_name(folder).as_deref());
    (!titles.is_empty()).then_some(titles)
}

/// The disc's own name: the Blu-ray title from META/DL/bdmt_eng.xml, else the volume label.
/// Disc titles on the disc itself are only a title, not the film's name, so the playlist or title set still follows.
fn disc_name(folder: &Path) -> Option<String> {
    let root = if folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case("BDMV")) {
        folder.parent()
    } else {
        Some(folder)
    };
    if let Some(root) = root {
        if let Ok(xml) = std::fs::read_to_string(root.join("META").join("DL").join("bdmt_eng.xml")) {
            if let Some(name) = xml_tag(&xml, "di:name") {
                return Some(name);
            }
        }
    }
    drives::volume_label(folder)
}

/// The text inside the first `<tag>...</tag>` pair, or None when it is missing or empty.
fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    let text = xml[start..end].trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// "Disc · Part", or just "Part" when the disc has no name.
fn with_disc(disc: Option<&str>, part: String) -> String {
    match disc {
        Some(d) => format!("{d} · {part}"),
        None => part,
    }
}

/// The BDMV folder, whether the user picked the disc root or the BDMV folder itself.
fn blu_ray_root(folder: &Path) -> Option<PathBuf> {
    let bdmv = if folder.join("BDMV").is_dir() { folder.join("BDMV") } else { folder.to_path_buf() };
    bdmv.join("PLAYLIST").is_dir().then_some(bdmv)
}

/// One title per playlist. Playlists whose clips are missing, and exact repeats, are left out.
fn blu_ray_titles(bdmv: &Path, disc: Option<&str>) -> Option<Vec<FoundTitle>> {
    let entries = std::fs::read_dir(bdmv.join("PLAYLIST")).ok()?;
    let mut lists: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mpls")))
        .collect();
    lists.sort();

    let mut seen: Vec<Vec<PathBuf>> = Vec::new();
    let mut titles = Vec::new();
    for list in lists {
        let Ok(bytes) = std::fs::read(&list) else { continue };
        let Some(items) = logic::parse_mpls(&bytes) else { continue };
        if items.is_empty() {
            continue;
        }
        let sources: Vec<PathBuf> = items
            .iter()
            .map(|i| bdmv.join("STREAM").join(format!("{}.m2ts", i.clip)))
            .collect();
        if !sources.iter().all(|p| p.is_file()) || seen.contains(&sources) {
            continue;
        }
        let duration: f64 = items
            .iter()
            .map(|i| logic::ticks_to_secs(i.out_time.saturating_sub(i.in_time)))
            .sum();
        let size_bytes = sources.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let stem = list.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        seen.push(sources.clone());
        titles.push(FoundTitle {
            name: with_disc(disc, format!("Playlist {stem}")),
            sources,
            duration_secs: Some(duration),
            size_bytes,
        });
    }
    (!titles.is_empty()).then_some(titles)
}

/// The VIDEO_TS folder, whether the user picked the disc root or the folder itself.
fn dvd_root(folder: &Path) -> Option<PathBuf> {
    if folder.join("VIDEO_TS").is_dir() {
        Some(folder.join("VIDEO_TS"))
    } else if folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case("VIDEO_TS")) {
        Some(folder.to_path_buf())
    } else {
        None
    }
}

/// One title per title set: its VTS_NN_1.VOB, VTS_NN_2.VOB ... parts joined in order.
fn dvd_titles(video_ts: &Path, disc: Option<&str>) -> Vec<FoundTitle> {
    let Ok(entries) = std::fs::read_dir(video_ts) else { return Vec::new() };
    let mut sets: BTreeMap<u32, Vec<(u32, PathBuf)>> = BTreeMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((set, part)) = logic::parse_vob_name(&name) {
            // Part 0 is the menu of the set, so only parts 1 and up are titles.
            if part >= 1 {
                sets.entry(set).or_default().push((part, entry.path()));
            }
        }
    }
    sets.into_iter()
        .map(|(set, mut parts)| {
            parts.sort();
            let sources: Vec<PathBuf> = parts.into_iter().map(|(_, p)| p).collect();
            let size_bytes = sources.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
            FoundTitle { name: with_disc(disc, format!("Title set {set:02}")), sources, duration_secs: None, size_bytes }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dr_disc_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn mpls(clip: &str, out_ticks: u32) -> Vec<u8> {
        let mut b = b"MPLS0200".to_vec();
        b.extend_from_slice(&20u32.to_be_bytes());
        b.resize(20, 0);
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0]); // one item, no sub paths
        b.extend_from_slice(&20u16.to_be_bytes());
        b.extend_from_slice(clip.as_bytes());
        b.extend_from_slice(b"M2TS");
        b.extend_from_slice(&[0, 0, 0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&out_ticks.to_be_bytes());
        b
    }

    #[test]
    fn blu_ray_playlist_becomes_one_title() {
        let root = temp("bd");
        std::fs::create_dir_all(root.join("BDMV/PLAYLIST")).unwrap();
        std::fs::create_dir_all(root.join("BDMV/STREAM")).unwrap();
        std::fs::write(root.join("BDMV/PLAYLIST/00800.mpls"), mpls("00001", 45_000 * 90)).unwrap();
        std::fs::write(root.join("BDMV/STREAM/00001.m2ts"), b"x").unwrap();
        let titles = find_disc_titles(&root).unwrap();
        assert_eq!(titles.len(), 1);
        // The name may start with a disc name taken from the volume, so only the end is fixed.
        assert!(titles[0].name.ends_with("Playlist 00800"));
        assert_eq!(titles[0].duration_secs, Some(90.0));
        assert_eq!(titles[0].sources.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dvd_parts_join_into_one_title() {
        let root = temp("dvd");
        let video_ts = root.join("VIDEO_TS");
        std::fs::create_dir_all(&video_ts).unwrap();
        for name in ["VIDEO_TS.VOB", "VTS_01_0.VOB", "VTS_01_2.VOB", "VTS_01_1.VOB"] {
            std::fs::write(video_ts.join(name), b"x").unwrap();
        }
        let titles = find_disc_titles(&root).unwrap();
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].sources.len(), 2);
        assert!(titles[0].sources[0].ends_with("VTS_01_1.VOB"));
        assert!(titles[0].sources[1].ends_with("VTS_01_2.VOB"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plain_folder_is_not_a_disc() {
        let root = temp("plain");
        std::fs::create_dir_all(&root).unwrap();
        assert!(find_disc_titles(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disc_name_comes_from_bluray_xml() {
        let xml = "<disclib><di:name>  Blue Movie  </di:name></disclib>";
        assert_eq!(xml_tag(xml, "di:name"), Some("Blue Movie".to_string()));
        assert_eq!(xml_tag("<di:name></di:name>", "di:name"), None);
        assert_eq!(xml_tag("nothing here", "di:name"), None);
    }
}
