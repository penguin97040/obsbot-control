//! Windows: whether another app is using a camera right now, read from the records
//! Windows keeps for its camera privacy indicator. Read-only registry access: no
//! camera is opened, nothing is written and nothing leaves the computer.

use std::sync::OnceLock;

use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, MAX_PATH};
use windows::Win32::Storage::FileSystem::GetLongPathNameW;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_QUERY_VALUE, KEY_READ, REG_QWORD, REG_VALUE_TYPE,
};

use super::CameraUse;
use crate::compact;

const WEBCAM_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam";

/// Closes a registry key when dropped.
struct Key(HKEY);

impl Key {
    fn open(parent: HKEY, name: &str, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<Key> {
        let mut key = HKEY::default();
        let name = HSTRING::from(name);
        // SAFETY: valid parent key, NUL-terminated name and out pointer.
        let r = unsafe { RegOpenKeyExW(parent, &name, None, access, &mut key) };
        (r == ERROR_SUCCESS).then_some(Key(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by RegOpenKeyExW and is closed once.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn read_time(key: &Key, name: PCWSTR) -> u64 {
    let mut value = 0u64;
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = std::mem::size_of::<u64>() as u32;
    // SAFETY: the data pointer has room for `size` bytes.
    let r = unsafe {
        RegQueryValueExW(key.0, name, None, Some(&mut kind), Some(&mut value as *mut u64 as *mut u8), Some(&mut size))
    };
    if r != ERROR_SUCCESS || kind != REG_QWORD || size as usize != std::mem::size_of::<u64>() {
        return 0;
    }
    value
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

/// This exe's path as Windows records it: the long form, in case it was started
/// through a short (8.3) path. Worked out once.
fn self_path() -> &'static str {
    static SELF: OnceLock<String> = OnceLock::new();
    SELF.get_or_init(|| {
        let mut path = [0u16; MAX_PATH as usize * 2];
        // SAFETY: the buffer and its length come from the same slice.
        let n = unsafe { GetModuleFileNameW(None, &mut path) } as usize;
        if n == 0 || n >= path.len() {
            return String::new();
        }
        let mut long = [0u16; MAX_PATH as usize * 2];
        // SAFETY: `path` is NUL-terminated (n < its length) and `long` is a writable buffer.
        let m = unsafe { GetLongPathNameW(PCWSTR(path.as_ptr()), Some(&mut long)) } as usize;
        if m > 0 && m < long.len() {
            String::from_utf16_lossy(&long[..m])
        } else {
            String::from_utf16_lossy(&path[..n])
        }
    })
}

/// Whether a process with this exe file name is running. Used to skip a record left
/// behind by an app that crashed while using the camera (Windows never wrote its stop
/// time). If the process list can't be read, the record is trusted.
fn is_running(exe_name: &str) -> bool {
    if exe_name.is_empty() {
        return true;
    }
    // SAFETY: a process snapshot; closed below.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return true;
    };
    let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut running = false;
    // SAFETY: entry.dwSize is set as the API requires.
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok && !running {
        running = wide_to_string(&entry.szExeFile).to_lowercase() == exe_name.to_lowercase();
        // SAFETY: as above, same snapshot and entry.
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the snapshot handle is ours and closed once.
    let _ = unsafe { CloseHandle(snapshot) };
    running
}

/// Looks through the app keys under `parent`; on a match returns the app's name.
fn find_user(parent: &Key, packaged: bool, self_path: &str) -> Option<String> {
    let mut name = [0u16; 256]; // registry key names are at most 255 characters
    for i in 0..4096u32 {
        let mut len = name.len() as u32;
        // SAFETY: the name buffer holds `len` UTF-16 units; the other outputs are unused.
        let r = unsafe { RegEnumKeyExW(parent.0, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if r == ERROR_NO_MORE_ITEMS {
            break;
        }
        if r != ERROR_SUCCESS {
            continue;
        }
        let key = String::from_utf16_lossy(&name[..(len as usize).min(name.len())]);
        if if packaged { key == "NonPackaged" } else { compact::is_same_path(&key, self_path) } {
            continue;
        }
        let Some(sub) = Key::open(parent.0, &key, KEY_QUERY_VALUE) else {
            continue;
        };
        let in_use = compact::in_use(
            read_time(&sub, windows::core::w!("LastUsedTimeStart")),
            read_time(&sub, windows::core::w!("LastUsedTimeStop")),
        );
        drop(sub);
        if in_use && (packaged || is_running(&compact::exe_file_name(&key))) {
            return Some(compact::app_name(&key, packaged));
        }
    }
    None
}

/// Skips this app's own use (the preview) and records left by desktop apps that are
/// no longer running. Takes a millisecond or so.
pub fn query() -> CameraUse {
    let self_path = self_path();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Some(webcam) = Key::open(hive, WEBCAM_KEY, KEY_READ) else {
            continue;
        };
        let mut found =
            Key::open(webcam.0, "NonPackaged", KEY_READ).and_then(|desktop| find_user(&desktop, false, self_path));
        if found.is_none() {
            found = find_user(&webcam, true, self_path);
        }
        if let Some(app_name) = found {
            return CameraUse { in_use: true, app_name };
        }
    }
    CameraUse::default()
}
