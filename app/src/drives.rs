//! Windows drive helpers: which drives are optical, disc names, free space and ejecting.

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

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
        pub fn GetDiskFreeSpaceExW(
            dir: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
        pub fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        pub fn DeviceIoControl(
            device: *mut c_void,
            code: u32,
            in_buf: *const c_void,
            in_len: u32,
            out_buf: *mut c_void,
            out_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
    }
}

/// Windows reports optical drives, Blu-ray included, with this type.
#[cfg(windows)]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(windows)]
const DRIVE_CDROM: u32 = 5;
#[cfg(windows)]
const SEM_FAILCRITICALERRORS: u32 = 1;

#[cfg(windows)]
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

/// Every drive root that is optical, or removable (some USB Blu-ray drives are reported that way).
/// The flag is true for optical drives.
#[cfg(windows)]
pub fn drive_roots() -> Vec<(PathBuf, bool)> {
    let mask = unsafe { sys::GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", char::from(b'A' + i as u8));
            match unsafe { sys::GetDriveTypeW(wide(root.as_ref()).as_ptr()) } {
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
    // The volume is the drive root, so use the last ancestor of the path.
    let root = wide(path.ancestors().last()?.as_os_str());
    let mut name = [0u16; 261];
    let ok = unsafe {
        sys::GetVolumeInformationW(
            root.as_ptr(),
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

/// Bytes free to this user on the drive that holds `path`.
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    let dir = wide(path.as_os_str());
    let mut free = 0u64;
    let ok = unsafe {
        sys::GetDiskFreeSpaceExW(
            dir.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// Asks the drive at `root` to eject its disc. Returns false when Windows refuses.
#[cfg(windows)]
pub fn eject(root: &Path) -> bool {
    const GENERIC_READ: u32 = 0x8000_0000;
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const OPEN_EXISTING: u32 = 3;
    const IOCTL_STORAGE_EJECT_MEDIA: u32 = 0x002D_4808;
    let Some(letter) = root.to_string_lossy().chars().next() else {
        return false;
    };
    let device = wide(format!("\\\\.\\{letter}:").as_ref());
    unsafe {
        let handle = sys::CreateFileW(
            device.as_ptr(),
            GENERIC_READ,
            SHARE_READ_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle.is_null() || handle as isize == -1 {
            return false;
        }
        let mut returned = 0u32;
        let ok = sys::DeviceIoControl(
            handle,
            IOCTL_STORAGE_EJECT_MEDIA,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
        sys::CloseHandle(handle);
        ok != 0
    }
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

#[cfg(not(windows))]
pub fn free_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub fn eject(_root: &Path) -> bool {
    false
}
