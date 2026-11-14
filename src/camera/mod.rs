//! Camera access: the types shared by every platform, the backend interface each
//! platform implements, and the background worker that owns the open camera.

pub mod demo;
pub mod worker;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use crate::protocol::{Model, XuBuffer};

/// One OBSBOT camera found on the system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Identifies the camera to the backend and the preview: the DirectShow device
    /// path on Windows (\\?\usb#vid_3564&pid_fef9&mi_00#...), the V4L2 capture node
    /// on Linux (/dev/video0), or "demo" for the built-in demo camera.
    pub path: String,
    /// The name the system reports.
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub model: Model,
}

/// Standard UVC controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UvcCtl {
    Pan,
    Tilt,
    Zoom,
    Exposure,
    Focus,
    Brightness,
    Contrast,
    Hue,
    Saturation,
    Sharpness,
    Gamma,
    WhiteBalance,
    BacklightComp,
    Gain,
    PowerLine,
}

impl UvcCtl {
    pub const COUNT: usize = 15;
    pub const ALL: [UvcCtl; UvcCtl::COUNT] = [
        UvcCtl::Pan,
        UvcCtl::Tilt,
        UvcCtl::Zoom,
        UvcCtl::Exposure,
        UvcCtl::Focus,
        UvcCtl::Brightness,
        UvcCtl::Contrast,
        UvcCtl::Hue,
        UvcCtl::Saturation,
        UvcCtl::Sharpness,
        UvcCtl::Gamma,
        UvcCtl::WhiteBalance,
        UvcCtl::BacklightComp,
        UvcCtl::Gain,
        UvcCtl::PowerLine,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Lower-case name for log messages.
    pub fn log_name(self) -> &'static str {
        match self {
            UvcCtl::Pan => "pan",
            UvcCtl::Tilt => "tilt",
            UvcCtl::Zoom => "zoom",
            UvcCtl::Exposure => "exposure",
            UvcCtl::Focus => "focus",
            UvcCtl::Brightness => "brightness",
            UvcCtl::Contrast => "contrast",
            UvcCtl::Hue => "hue",
            UvcCtl::Saturation => "saturation",
            UvcCtl::Sharpness => "sharpness",
            UvcCtl::Gamma => "gamma",
            UvcCtl::WhiteBalance => "white balance",
            UvcCtl::BacklightComp => "backlight compensation",
            UvcCtl::Gain => "gain",
            UvcCtl::PowerLine => "anti-flicker",
        }
    }

    /// Short name for the diagnostics report.
    pub fn diag_name(self) -> &'static str {
        match self {
            UvcCtl::Pan => "Pan",
            UvcCtl::Tilt => "Tilt",
            UvcCtl::Zoom => "Zoom",
            UvcCtl::Exposure => "Exposure",
            UvcCtl::Focus => "Focus",
            UvcCtl::Brightness => "Brightness",
            UvcCtl::Contrast => "Contrast",
            UvcCtl::Hue => "Hue",
            UvcCtl::Saturation => "Saturation",
            UvcCtl::Sharpness => "Sharpness",
            UvcCtl::Gamma => "Gamma",
            UvcCtl::WhiteBalance => "WhiteBalance",
            UvcCtl::BacklightComp => "BacklightComp",
            UvcCtl::Gain => "Gain",
            UvcCtl::PowerLine => "PowerLine",
        }
    }
}

/// A UVC control's range and current value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UvcInfo {
    pub supported: bool,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub def: i32,
    pub can_auto: bool,
    pub can_manual: bool,
    pub value: i32,
    pub is_auto: bool,
}

impl Default for UvcInfo {
    fn default() -> Self {
        UvcInfo {
            supported: false,
            min: 0,
            max: 0,
            step: 1,
            def: 0,
            can_auto: false,
            can_manual: true,
            value: 0,
            is_auto: false,
        }
    }
}

/// Anti-flicker values for UvcCtl::PowerLine (the UVC power line frequency control;
/// Linux uses the same numbers).
pub const POWER_LINE_DISABLED: i32 = 0;
pub const POWER_LINE_50HZ: i32 = 1;
pub const POWER_LINE_60HZ: i32 = 2;

/// What a platform must provide to control a camera. Every method is called from
/// the camera worker's thread only, and the backend is created on that thread, so
/// implementations need not be thread-safe. Errors are short, readable messages
/// such as "0x80070005 (Access is denied.)" or "EBUSY (Device or resource busy)".
pub trait CameraBackend {
    /// Called once on the worker thread before anything else (Windows starts COM here).
    fn thread_start(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Called once on the worker thread after everything else.
    fn thread_end(&mut self) {}

    /// Lists the OBSBOT cameras (USB vendor ID 0x3564) that are plugged in.
    fn enumerate(&mut self) -> Vec<DeviceInfo>;
    /// Opens a camera (closing any open one first). Logs why on failure.
    fn open(&mut self, info: &DeviceInfo) -> bool;
    fn close(&mut self);
    fn is_open(&self) -> bool;

    /// Whether the OBSBOT extension unit was found on the open camera.
    fn has_xu(&self) -> bool;
    /// Where the extension unit was found: the KS node (Windows) or UVC unit ID (Linux).
    fn xu_location(&self) -> u32;

    /// The range and current value of a control, or None when it isn't supported.
    fn query_uvc(&mut self, control: UvcCtl) -> Option<UvcInfo>;
    /// The current value and whether automatic mode is on.
    fn get_uvc(&mut self, control: UvcCtl) -> Option<(i32, bool)>;
    /// Sets a value (clamped to the range) or turns automatic mode on or off.
    fn set_uvc(&mut self, control: UvcCtl, value: i32, auto: bool) -> Result<(), String>;

    /// SET_CUR of 60 bytes on an extension-unit selector.
    fn xu_set(&mut self, selector: u8, data: &XuBuffer) -> Result<(), String>;
    /// GET_CUR of 60 bytes. Fails unless all 60 bytes arrive.
    fn xu_get(&mut self, selector: u8) -> Result<XuBuffer, String>;
}

/// The real camera backend for this platform.
pub fn platform_backend() -> Box<dyn CameraBackend> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxCamera::new())
    }
    #[cfg(windows)]
    {
        Box::new(windows::WindowsCamera::new())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Box::new(demo::NoCamera)
    }
}
