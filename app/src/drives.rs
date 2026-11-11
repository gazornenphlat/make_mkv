//! Finds the CD, DVD and Blu-ray drives Windows knows about, and reads disc volume labels.

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod sys {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetLogicalDrives() -> u32;
        pub fn GetDriveTypeW(root: *const u16) -> u32;
        pub fn SetErrorMode(mode: u32) -> u32;
        pub fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_len: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            fs_name: *mut u16,
            fs_name_len: u32,
        ) -> i32;
    }
}

/// Windows reports optical drives, Blu-ray included, with this type.
#[cfg(windows)]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(windows)]
const DRIVE_CDROM: u32 = 5;
#[cfg(windows)]
const SEM_FAILCRITICALERRORS: u32 = 1;

/// Every drive root that is optical, or removable (some USB Blu-ray drives are reported that way).
/// The flag is true for optical drives.
#[cfg(windows)]
pub fn drive_roots() -> Vec<(PathBuf, bool)> {
    let mask = unsafe { sys::GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", char::from(b'A' + i as u8));
            let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
            match unsafe { sys::GetDriveTypeW(wide.as_ptr()) } {
                DRIVE_CDROM => Some((PathBuf::from(root), true)),
                DRIVE_REMOVABLE => Some((PathBuf::from(root), false)),
                _ => None,
            }
        })
        .collect()
}

/// Stops Windows from showing an error box when an empty drive is looked at.
#[cfg(windows)]
pub fn quiet_drive_errors() {
    unsafe { sys::SetErrorMode(SEM_FAILCRITICALERRORS) };
}

/// The volume label (the disc's name) of the drive that holds `path`, when it has one.
#[cfg(windows)]
pub fn volume_label(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    // The volume is the drive root, so use the last ancestor of the path.
    let root = path.ancestors().last()?;
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut name = [0u16; 261];
    let ok = unsafe {
        sys::GetVolumeInformationW(
            wide.as_ptr(),
            name.as_mut_ptr(),
            name.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    let label = String::from_utf16_lossy(&name[..len]).trim().to_string();
    (!label.is_empty()).then_some(label)
}

#[cfg(not(windows))]
pub fn drive_roots() -> Vec<(PathBuf, bool)> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn quiet_drive_errors() {}

#[cfg(not(windows))]
pub fn volume_label(_path: &Path) -> Option<String> {
    None
}
