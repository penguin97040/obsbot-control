//! A simulated Tiny 2 Lite, started with `--demo`. It lets the app be tried, tested
//! and screenshotted without a camera. Nothing here touches any hardware.

use super::{CameraBackend, DeviceInfo, UvcCtl, UvcInfo, POWER_LINE_50HZ};
use crate::preview::demo::DEMO_PATH;
use crate::protocol::{self, Model, XuBuffer, PID_TINY2_LITE, VENDOR_ID};

/// The demo camera's controls, ranges and status, as a real Tiny 2 Lite reports them.
pub struct DemoCamera {
    open: bool,
    uvc: [UvcInfo; UvcCtl::COUNT],
    status: XuBuffer,
    reply: XuBuffer,
}

fn info(min: i32, max: i32, step: i32, def: i32, can_auto: bool, is_auto: bool) -> UvcInfo {
    UvcInfo { supported: true, min, max, step, def, can_auto, can_manual: true, value: def, is_auto }
}

impl DemoCamera {
    pub fn new() -> Self {
        let mut uvc = [UvcInfo::default(); UvcCtl::COUNT];
        uvc[UvcCtl::Pan.index()] = info(-468_000, 468_000, 3600, 0, false, false);
        uvc[UvcCtl::Tilt.index()] = info(-324_000, 324_000, 3600, 0, false, false);
        uvc[UvcCtl::Zoom.index()] = info(0, 100, 1, 0, false, false);
        uvc[UvcCtl::Exposure.index()] = info(-11, -2, 1, -6, true, true);
        uvc[UvcCtl::Focus.index()] = info(0, 100, 1, 50, true, true);
        uvc[UvcCtl::Brightness.index()] = info(0, 100, 1, 50, false, false);
        uvc[UvcCtl::Contrast.index()] = info(0, 100, 1, 50, false, false);
        uvc[UvcCtl::Hue.index()] = info(-180, 180, 1, 0, false, false);
        uvc[UvcCtl::Saturation.index()] = info(0, 100, 1, 50, false, false);
        uvc[UvcCtl::Sharpness.index()] = info(0, 100, 1, 50, false, false);
        uvc[UvcCtl::Gamma.index()] = info(100, 500, 1, 300, false, false);
        uvc[UvcCtl::WhiteBalance.index()] = info(2000, 10000, 100, 5000, true, true);
        uvc[UvcCtl::BacklightComp.index()] = info(0, 2, 1, 0, false, false);
        uvc[UvcCtl::Gain.index()] = info(0, 100, 1, 0, false, false);
        uvc[UvcCtl::PowerLine.index()] = info(0, 2, 1, POWER_LINE_50HZ, false, false);
        uvc[UvcCtl::Pan.index()].value = 36_000;
        uvc[UvcCtl::Tilt.index()].value = -14_400;
        uvc[UvcCtl::Zoom.index()].value = 20;
        let mut status = [0u8; protocol::XU_LENGTH];
        status[0x09] = 1; // running
        status[0x11] = 1; // wide (Lite code)
        let (m, n) = protocol::AiMode::UpperBody.bytes();
        status[0x18] = m;
        status[0x1C] = n;
        DemoCamera { open: false, uvc, status, reply: [0u8; protocol::XU_LENGTH] }
    }

    pub fn device() -> DeviceInfo {
        DeviceInfo {
            path: DEMO_PATH.to_string(),
            name: "OBSBOT Tiny 2 Lite (demo)".to_string(),
            vid: VENDOR_ID,
            pid: PID_TINY2_LITE,
            model: Model::Tiny2Lite,
        }
    }

    /// Updates the status block the way the camera would after a simple command.
    fn apply_simple(&mut self, data: &XuBuffer) {
        match (data[0], data[1]) {
            (0x16, 0x02) => {
                self.status[0x18] = data[2];
                self.status[0x1C] = data[3];
            }
            (0x01, 0x01) => self.status[0x06] = data[2],
            (0x04, 0x01) => self.status[0x11] = data[2],
            (0x02, 0x01) => {
                self.status[0x02] = data[2];
                self.status[0x09] = if data[2] == 1 { 3 } else { 1 };
            }
            _ => {}
        }
    }

    /// Acknowledges a framed command the way the camera does, and applies it.
    fn apply_framed(&mut self, data: &XuBuffer) {
        let Some(frame) = protocol::parse_frame(data) else { return };
        match frame.command {
            protocol::cmd::SLEEP_WAKE if !frame.payload.is_empty() => {
                let sleep = frame.payload[0] == 1;
                self.status[0x02] = sleep as u8;
                self.status[0x09] = if sleep { 3 } else { 1 };
            }
            protocol::cmd::TRACKING_SPEED if !frame.payload.is_empty() => {
                self.status[0x24] = frame.payload[0];
            }
            protocol::cmd::RECENTRE => {
                self.uvc[UvcCtl::Pan.index()].value = 0;
                self.uvc[UvcCtl::Tilt.index()].value = 0;
            }
            _ => {}
        }
        let receiver = match data[9] {
            0x02 => protocol::Receiver::Camera,
            0x03 => protocol::Receiver::Gimbal,
            _ => protocol::Receiver::Ai,
        };
        let mut reply = protocol::build_frame(frame.seq, receiver, frame.command, &[]);
        reply[1] = 0x05;
        reply[6] = 0;
        reply[7] = 0;
        let crc = protocol::crc16_usb(&reply[0..12]);
        reply[6..8].copy_from_slice(&crc.to_le_bytes());
        self.reply = reply;
    }
}

impl Default for DemoCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraBackend for DemoCamera {
    fn enumerate(&mut self) -> Vec<DeviceInfo> {
        vec![DemoCamera::device()]
    }

    fn open(&mut self, info: &DeviceInfo) -> bool {
        self.open = info.path == DEMO_PATH;
        self.open
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn has_xu(&self) -> bool {
        self.open
    }

    fn xu_location(&self) -> u32 {
        protocol::XU_DEFAULT_UNIT as u32
    }

    fn query_uvc(&mut self, control: UvcCtl) -> Option<UvcInfo> {
        Some(self.uvc[control.index()]).filter(|u| u.supported && self.open)
    }

    fn get_uvc(&mut self, control: UvcCtl) -> Option<(i32, bool)> {
        self.query_uvc(control).map(|u| (u.value, u.is_auto))
    }

    fn set_uvc(&mut self, control: UvcCtl, value: i32, auto: bool) -> Result<(), String> {
        let u = &mut self.uvc[control.index()];
        if !self.open || !u.supported {
            return Err("not supported".into());
        }
        if auto && !u.can_auto {
            return Err("automatic mode isn't supported".into());
        }
        u.value = value.clamp(u.min, u.max);
        u.is_auto = auto;
        if control == UvcCtl::Zoom {
            self.status[0x04..0x06].copy_from_slice(&((100 + u.value * 3) as u16).to_le_bytes());
        }
        Ok(())
    }

    fn xu_set(&mut self, selector: u8, data: &XuBuffer) -> Result<(), String> {
        if !self.open {
            return Err("not open".into());
        }
        match selector {
            protocol::SELECTOR_SIMPLE => self.apply_simple(data),
            protocol::SELECTOR_FRAMED => self.apply_framed(data),
            _ => {}
        }
        Ok(())
    }

    fn xu_get(&mut self, selector: u8) -> Result<XuBuffer, String> {
        if !self.open {
            return Err("not open".into());
        }
        Ok(match selector {
            protocol::SELECTOR_SIMPLE => self.status,
            protocol::SELECTOR_FRAMED => self.reply,
            protocol::SELECTOR_PRODUCT_NAME => protocol::build_simple(b"OBSBOT Tiny 2 Lite"),
            _ => [0u8; protocol::XU_LENGTH],
        })
    }
}

/// Used on systems without a camera backend: never finds a camera.
#[allow(dead_code)]
pub struct NoCamera;

impl CameraBackend for NoCamera {
    fn enumerate(&mut self) -> Vec<DeviceInfo> {
        Vec::new()
    }
    fn open(&mut self, _info: &DeviceInfo) -> bool {
        false
    }
    fn close(&mut self) {}
    fn is_open(&self) -> bool {
        false
    }
    fn has_xu(&self) -> bool {
        false
    }
    fn xu_location(&self) -> u32 {
        0
    }
    fn query_uvc(&mut self, _control: UvcCtl) -> Option<UvcInfo> {
        None
    }
    fn get_uvc(&mut self, _control: UvcCtl) -> Option<(i32, bool)> {
        None
    }
    fn set_uvc(&mut self, _control: UvcCtl, _value: i32, _auto: bool) -> Result<(), String> {
        Err("no camera".into())
    }
    fn xu_set(&mut self, _selector: u8, _data: &XuBuffer) -> Result<(), String> {
        Err("no camera".into())
    }
    fn xu_get(&mut self, _selector: u8) -> Result<XuBuffer, String> {
        Err("no camera".into())
    }
}
