//! Is another app using the camera? Used to switch to compact view (no preview)
//! while a video call or similar has the camera, and back again afterwards.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraUse {
    pub in_use: bool,
    /// A readable name for the app using it, e.g. "Zoom". May be empty.
    pub app_name: String,
}

/// Checks whether an app other than this one is using a camera. `device_path` is the
/// connected camera's DeviceInfo::path (empty when none). `our_preview_running` is
/// true while this app's own preview is streaming, in which case nobody else can be.
/// Must be quick (well under 50 ms): it runs on the interface thread every 2 seconds.
pub fn query(device_path: &str, our_preview_running: bool) -> CameraUse {
    if device_path == crate::preview::demo::DEMO_PATH {
        return CameraUse::default();
    }
    #[cfg(target_os = "linux")]
    {
        linux::query(device_path, our_preview_running)
    }
    #[cfg(windows)]
    {
        let _ = (device_path, our_preview_running);
        windows::query()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (device_path, our_preview_running);
        CameraUse::default()
    }
}
