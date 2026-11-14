//! Windows services: single instance mutex, dark title bar and the Windows version.

use raw_window_handle::RawWindowHandle;
use windows::core::{s, w, BOOL};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};

use super::InstanceGuard;

/// Owns the single-instance mutex; closing it lets a later copy start.
struct Mutex(HANDLE);

impl Drop for Mutex {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// None when another copy of the app is already running (and brings it forward).
pub fn single_instance() -> Option<InstanceGuard> {
    // One instance only: two copies would fight over the camera.
    // SAFETY: a named mutex with default security attributes.
    let (mutex, already) = match unsafe { CreateMutexW(None, true, w!("Local\\obsbot-control-single-instance")) } {
        // SAFETY: read straight after CreateMutexW, before any other API call.
        Ok(h) => (Some(Mutex(h)), unsafe { GetLastError() } == ERROR_ALREADY_EXISTS),
        Err(_) => (None, false),
    };
    if already {
        // SAFETY: looking up a top-level window by its exact title.
        if let Ok(existing) = unsafe { FindWindowW(None, w!("obsbot-control")) } {
            // SAFETY: plain calls on a window handle Windows just gave us.
            unsafe {
                // Only when minimised: SW_RESTORE would un-maximise it.
                if IsIconic(existing).as_bool() {
                    let _ = ShowWindow(existing, SW_RESTORE);
                }
                let _ = SetForegroundWindow(existing);
            }
        }
        return None;
    }
    Some(InstanceGuard::new(mutex))
}

/// Gives a new window a dark title bar (DWMWA_USE_IMMERSIVE_DARK_MODE).
pub fn style_window(handle: RawWindowHandle) {
    let RawWindowHandle::Win32(win32) = handle else { return };
    let hwnd = HWND(win32.hwnd.get() as *mut _);
    let dark = BOOL::from(true);
    let set = |attribute: DWMWINDOWATTRIBUTE| {
        // SAFETY: a BOOL-sized attribute value that outlives the call.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                &dark as *const BOOL as *const _,
                std::mem::size_of::<BOOL>() as u32,
            )
        }
    };
    // Windows 10 before 20H1 used attribute 19 for the same thing.
    if set(DWMWA_USE_IMMERSIVE_DARK_MODE).is_err() {
        let _ = set(DWMWINDOWATTRIBUTE(19));
    }
}

/// e.g. "Windows 10.0 build 22631"
pub fn os_version() -> String {
    // RtlGetVersion reports the real version; GetVersionEx lies without a manifest entry.
    type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
    let mut v = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: ntdll is always loaded; the looked-up export has the RtlGetVersion
    // signature, and `v` has its size field set as it requires.
    let ok = unsafe {
        GetModuleHandleW(w!("ntdll.dll"))
            .ok()
            .and_then(|ntdll| GetProcAddress(ntdll, s!("RtlGetVersion")))
            .is_some_and(|f| {
                let f: RtlGetVersionFn = std::mem::transmute(f);
                f(&mut v) == 0
            })
    };
    if ok {
        format!(
            "Windows {}.{} build {}",
            v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber
        )
    } else {
        "Windows (unknown version)".to_string()
    }
}
