//! Linux camera backend: V4L2 controls and the UVC extension unit (UVCIOC_CTRL_QUERY).

use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::{CameraBackend, DeviceInfo, UvcCtl, UvcInfo};
use crate::protocol::{Model, XuBuffer, SELECTOR_SIMPLE, VENDOR_ID, XU_DEFAULT_UNIT, XU_GUID_BYTES, XU_LENGTH};
use crate::v4l2;

const SYSFS_VIDEO: &str = "/sys/class/video4linux";

/// Which V4L2 controls stand behind one of our controls.
struct CtlIds {
    value: u32,
    /// The paired automatic-mode control, if there is one.
    auto: Option<u32>,
}

fn control_ids(control: UvcCtl) -> CtlIds {
    let (value, auto) = match control {
        UvcCtl::Pan => (v4l2::CID_PAN_ABSOLUTE, None),
        UvcCtl::Tilt => (v4l2::CID_TILT_ABSOLUTE, None),
        UvcCtl::Zoom => (v4l2::CID_ZOOM_ABSOLUTE, None),
        UvcCtl::Exposure => (v4l2::CID_EXPOSURE_ABSOLUTE, Some(v4l2::CID_EXPOSURE_AUTO)),
        UvcCtl::Focus => (v4l2::CID_FOCUS_ABSOLUTE, Some(v4l2::CID_FOCUS_AUTO)),
        UvcCtl::Brightness => (v4l2::CID_BRIGHTNESS, None),
        UvcCtl::Contrast => (v4l2::CID_CONTRAST, None),
        UvcCtl::Hue => (v4l2::CID_HUE, Some(v4l2::CID_HUE_AUTO)),
        UvcCtl::Saturation => (v4l2::CID_SATURATION, None),
        UvcCtl::Sharpness => (v4l2::CID_SHARPNESS, None),
        UvcCtl::Gamma => (v4l2::CID_GAMMA, None),
        UvcCtl::WhiteBalance => (v4l2::CID_WHITE_BALANCE_TEMPERATURE, Some(v4l2::CID_AUTO_WHITE_BALANCE)),
        UvcCtl::BacklightComp => (v4l2::CID_BACKLIGHT_COMPENSATION, None),
        UvcCtl::Gain => (v4l2::CID_GAIN, None),
        UvcCtl::PowerLine => (v4l2::CID_POWER_LINE_FREQUENCY, None),
    };
    CtlIds { value, auto }
}

/// The value of the paired auto control that means "manual".
fn manual_value(auto_id: u32) -> i32 {
    // Exposure's auto control is a menu; the others are on/off switches.
    if auto_id == v4l2::CID_EXPOSURE_AUTO {
        v4l2::EXPOSURE_MANUAL
    } else {
        0
    }
}

/// Finds the OBSBOT extension unit's ID in a USB device's raw descriptors (the
/// device descriptor followed by its configuration descriptors, as sysfs gives
/// them). Only class-specific descriptors inside a VideoControl interface count:
/// subtype 6 means something else in a VideoStreaming interface (an MJPEG format).
pub(crate) fn find_xu_unit(descriptors: &[u8], guid: &[u8; 16]) -> Option<u8> {
    const TYPE_CONFIGURATION: u8 = 0x02;
    const TYPE_INTERFACE: u8 = 0x04;
    const TYPE_CS_INTERFACE: u8 = 0x24;
    const CLASS_VIDEO: u8 = 0x0E;
    const SUBCLASS_VIDEOCONTROL: u8 = 0x01;
    const VC_EXTENSION_UNIT: u8 = 0x06;
    // bLength, bDescriptorType, bDescriptorSubtype, bUnitID, guidExtensionCode[16],
    // bNumControls, bNrInPins, bControlSize, iExtension: at least 24 bytes.
    const XU_MIN_LENGTH: usize = 24;

    let mut pos = 0;
    let mut in_video_control = false;
    while pos + 2 <= descriptors.len() {
        let len = descriptors[pos] as usize;
        if len < 2 || pos + len > descriptors.len() {
            break; // Corrupt or cut short: stop rather than guess.
        }
        let d = &descriptors[pos..pos + len];
        match d[1] {
            TYPE_CONFIGURATION => in_video_control = false,
            TYPE_INTERFACE if len >= 9 => {
                in_video_control = d[5] == CLASS_VIDEO && d[6] == SUBCLASS_VIDEOCONTROL;
            }
            TYPE_CS_INTERFACE
                if in_video_control && len >= XU_MIN_LENGTH && d[2] == VC_EXTENSION_UNIT && d[4..20] == guid[..] =>
            {
                return Some(d[3]);
            }
            _ => {}
        }
        pos += len;
    }
    None
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn read_hex_u16(path: &Path) -> Option<u16> {
    u16::from_str_radix(&read_trimmed(path)?, 16).ok()
}

/// The USB device folder in sysfs behind a video4linux entry. Its `device` link
/// points at the USB interface; the device (with idVendor) is the folder above.
fn usb_device_dir(sysfs_video: &Path) -> Option<PathBuf> {
    let interface = fs::canonicalize(sysfs_video.join("device")).ok()?;
    if interface.join("idVendor").is_file() {
        return Some(interface);
    }
    let parent = interface.parent()?;
    parent.join("idVendor").is_file().then(|| parent.to_path_buf())
}

/// The sysfs entry for a device node, found by its device number so that a
/// symlink such as /dev/v4l/by-id/... works too.
fn sysfs_for_node(path: &str) -> Option<PathBuf> {
    let rdev = fs::metadata(path).ok()?.rdev();
    let link = format!("/sys/dev/char/{}:{}", libc::major(rdev), libc::minor(rdev));
    fs::canonicalize(link).ok()
}

/// Whether a node can capture video (uvcvideo also creates a metadata node per
/// camera, which can't). Returns the card name when it can.
fn capture_card(path: &str) -> Result<Option<String>, i32> {
    let fd = v4l2::open_node(path)?;
    let mut cap: v4l2::Capability = v4l2::zeroed();
    v4l2::ioctl(&fd, v4l2::VIDIOC_QUERYCAP, &mut cap)?;
    let caps = if cap.capabilities & v4l2::CAP_DEVICE_CAPS != 0 { cap.device_caps } else { cap.capabilities };
    Ok((caps & v4l2::CAP_VIDEO_CAPTURE != 0).then(|| v4l2::c_text(&cap.card)))
}

pub struct LinuxCamera {
    fd: Option<OwnedFd>,
    xu_unit: Option<u8>,
}

impl LinuxCamera {
    pub fn new() -> Self {
        LinuxCamera { fd: None, xu_unit: None }
    }

    fn query_ctrl(&self, id: u32) -> Option<v4l2::QueryCtrl> {
        let fd = self.fd.as_ref()?;
        let mut q: v4l2::QueryCtrl = v4l2::zeroed();
        q.id = id;
        v4l2::ioctl(fd, v4l2::VIDIOC_QUERYCTRL, &mut q).ok()?;
        (q.flags & v4l2::CTRL_FLAG_DISABLED == 0 && q.maximum >= q.minimum).then_some(q)
    }

    fn get_ctrl(&self, id: u32) -> Option<i32> {
        let fd = self.fd.as_ref()?;
        let mut c = v4l2::Control { id, value: 0 };
        v4l2::ioctl(fd, v4l2::VIDIOC_G_CTRL, &mut c).ok()?;
        Some(c.value)
    }

    fn set_ctrl(&self, id: u32, value: i32) -> Result<(), String> {
        let fd = self.fd.as_ref().ok_or("The camera isn't open.")?;
        let mut c = v4l2::Control { id, value };
        v4l2::ioctl(fd, v4l2::VIDIOC_S_CTRL, &mut c).map_err(v4l2::errno_text)
    }

    /// Whether the auto control is present and usable, and whether it is on now.
    fn auto_state(&self, auto_id: Option<u32>) -> Option<(v4l2::QueryCtrl, bool)> {
        let id = auto_id?;
        let q = self.query_ctrl(id)?;
        if q.flags & v4l2::CTRL_FLAG_READ_ONLY != 0 {
            return None;
        }
        let on = self.get_ctrl(id).is_some_and(|v| v != manual_value(id));
        Some((q, on))
    }

    /// Turns exposure's automatic mode on. UVC cameras usually offer only
    /// "aperture priority" as their automatic mode, so try the camera's own default
    /// first, then the automatic modes in turn.
    fn set_auto_exposure(&self, menu: &v4l2::QueryCtrl) -> Result<(), String> {
        let mut candidates = vec![menu.default_value];
        candidates.extend([v4l2::EXPOSURE_APERTURE_PRIORITY, v4l2::EXPOSURE_AUTO, v4l2::EXPOSURE_SHUTTER_PRIORITY]);
        let mut last_error = String::from("EINVAL (Invalid argument)");
        for mode in candidates {
            if mode == v4l2::EXPOSURE_MANUAL || mode < menu.minimum || mode > menu.maximum {
                continue;
            }
            match self.set_ctrl(v4l2::CID_EXPOSURE_AUTO, mode) {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    fn find_xu(&self, path: &str) -> Option<u8> {
        let fd = self.fd.as_ref()?;
        let from_descriptors = sysfs_for_node(path)
            .and_then(|sys| usb_device_dir(&sys))
            .and_then(|usb| fs::read(usb.join("descriptors")).ok())
            .and_then(|bytes| find_xu_unit(&bytes, &XU_GUID_BYTES));
        let mut candidates = Vec::new();
        candidates.extend(from_descriptors);
        if from_descriptors != Some(XU_DEFAULT_UNIT) {
            candidates.push(XU_DEFAULT_UNIT);
        }
        // Probe: a harmless read of the status block only succeeds on our unit.
        candidates.into_iter().find(|&unit| {
            let mut probe = [0u8; XU_LENGTH];
            v4l2::uvc_xu_query(fd, unit, SELECTOR_SIMPLE, v4l2::UVC_GET_CUR, &mut probe).is_ok()
        })
    }
}

impl Default for LinuxCamera {
    fn default() -> Self {
        Self::new()
    }
}

/// A name for /dev/videoN that survives replugging and restarts, so the chosen camera
/// is remembered: its /dev/v4l/by-id link (which carries the serial number), else its
/// by-path link (the USB port), else the node itself. Every user of the path opens or
/// stats it, so a symlink works anywhere a node does.
fn stable_path(node: &str) -> String {
    let target = Path::new(node);
    for dir in ["/dev/v4l/by-id", "/dev/v4l/by-path"] {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut links: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        links.sort();
        if let Some(link) = links.into_iter().find(|l| fs::canonicalize(l).ok().as_deref() == Some(target)) {
            return link.to_string_lossy().into_owned();
        }
    }
    node.to_string()
}

impl CameraBackend for LinuxCamera {
    fn enumerate(&mut self) -> Vec<DeviceInfo> {
        let Ok(entries) = fs::read_dir(SYSFS_VIDEO) else {
            return Vec::new();
        };
        let mut found: Vec<(u32, DeviceInfo)> = Vec::new();
        for entry in entries.flatten() {
            let sys = entry.path();
            let Some(number) = v4l2::node_number(&sys.to_string_lossy()) else {
                continue;
            };
            let Some(usb) = usb_device_dir(&sys) else {
                continue;
            };
            let vid = read_hex_u16(&usb.join("idVendor")).unwrap_or(0);
            if vid != VENDOR_ID {
                continue;
            }
            let pid = read_hex_u16(&usb.join("idProduct")).unwrap_or(0);
            let path = stable_path(&format!("/dev/video{number}"));
            let card = match capture_card(&path) {
                Ok(Some(card)) => card,
                Ok(None) => continue,
                // Can't open it (no permission, or busy): uvcvideo numbers the capture
                // node 0 and the metadata node 1, so list only index 0. Opening will
                // then explain the problem.
                Err(_) => {
                    if read_trimmed(&sys.join("index")).as_deref() != Some("0") {
                        continue;
                    }
                    String::new()
                }
            };
            let model = Model::from_pid(pid);
            let name = read_trimmed(&usb.join("product"))
                .or_else(|| (!card.is_empty()).then_some(card))
                .unwrap_or_else(|| model.name().to_string());
            found.push((number, DeviceInfo { path, name, vid, pid, model }));
        }
        found.sort_by_key(|(number, _)| *number);
        found.into_iter().map(|(_, info)| info).collect()
    }

    fn open(&mut self, info: &DeviceInfo) -> bool {
        self.close();
        let fd = match v4l2::open_node(&info.path) {
            Ok(fd) => fd,
            Err(e) => {
                crate::log_error!("Could not open the camera ({}): {}", info.path, v4l2::errno_text(e));
                if e == libc::EACCES {
                    crate::log_warn!("{}", v4l2::permission_hint(&info.path));
                }
                return false;
            }
        };
        self.fd = Some(fd);
        self.xu_unit = self.find_xu(&info.path);
        match self.xu_unit {
            Some(unit) => crate::log_info!("Found the OBSBOT extension unit on UVC unit {unit}."),
            None => crate::log_warn!(
                "OBSBOT extension unit not found: AI tracking, HDR, FOV and sleep controls are unavailable."
            ),
        }
        true
    }

    fn close(&mut self) {
        self.fd = None;
        self.xu_unit = None;
    }

    fn is_open(&self) -> bool {
        self.fd.is_some()
    }

    fn has_xu(&self) -> bool {
        self.xu_unit.is_some()
    }

    fn xu_location(&self) -> u32 {
        self.xu_unit.map_or(0, u32::from)
    }

    fn query_uvc(&mut self, control: UvcCtl) -> Option<UvcInfo> {
        let ids = control_ids(control);
        let q = self.query_ctrl(ids.value)?;
        let auto = self.auto_state(ids.auto);
        Some(UvcInfo {
            supported: true,
            min: q.minimum,
            max: q.maximum,
            step: q.step.max(1),
            def: q.default_value,
            can_auto: auto.is_some(),
            can_manual: q.flags & v4l2::CTRL_FLAG_READ_ONLY == 0,
            // Some controls can't be read back; show the default then.
            value: self.get_ctrl(ids.value).unwrap_or(q.default_value),
            is_auto: auto.is_some_and(|(_, on)| on),
        })
    }

    fn get_uvc(&mut self, control: UvcCtl) -> Option<(i32, bool)> {
        let ids = control_ids(control);
        let value = self.get_ctrl(ids.value)?;
        let is_auto = match ids.auto {
            Some(id) => self.get_ctrl(id).is_some_and(|v| v != manual_value(id)),
            None => false,
        };
        Some((value, is_auto))
    }

    fn set_uvc(&mut self, control: UvcCtl, value: i32, auto: bool) -> Result<(), String> {
        if self.fd.is_none() {
            return Err("The camera isn't open.".into());
        }
        let ids = control_ids(control);
        let q =
            self.query_ctrl(ids.value).ok_or_else(|| format!("This camera has no {} control.", control.log_name()))?;
        let auto_state = self.auto_state(ids.auto);
        if auto {
            let Some((menu, _)) = auto_state else {
                return Err(format!("This camera has no automatic {}.", control.log_name()));
            };
            return if ids.auto == Some(v4l2::CID_EXPOSURE_AUTO) {
                self.set_auto_exposure(&menu)
            } else {
                self.set_ctrl(menu.id, 1)
            };
        }
        // Switch to manual first: while automatic mode is on, the kernel refuses
        // (or ignores) a manual value.
        if let (Some(auto_id), Some((_, true))) = (ids.auto, auto_state) {
            self.set_ctrl(auto_id, manual_value(auto_id))?;
        }
        // Read-only or inactive controls are left to the kernel, which says why.
        self.set_ctrl(ids.value, value.clamp(q.minimum, q.maximum))
    }

    fn xu_set(&mut self, selector: u8, data: &XuBuffer) -> Result<(), String> {
        let (Some(fd), Some(unit)) = (self.fd.as_ref(), self.xu_unit) else {
            return Err("The OBSBOT extension unit isn't available.".into());
        };
        let mut copy = *data;
        v4l2::uvc_xu_query(fd, unit, selector, v4l2::UVC_SET_CUR, &mut copy).map_err(v4l2::errno_text)
    }

    fn xu_get(&mut self, selector: u8) -> Result<XuBuffer, String> {
        let (Some(fd), Some(unit)) = (self.fd.as_ref(), self.xu_unit) else {
            return Err("The OBSBOT extension unit isn't available.".into());
        };
        // The driver fails the request unless the camera returns all 60 bytes.
        let mut buf = [0u8; XU_LENGTH];
        v4l2::uvc_xu_query(fd, unit, selector, v4l2::UVC_GET_CUR, &mut buf).map_err(v4l2::errno_text)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: [u8; 16] = XU_GUID_BYTES;

    fn device_descriptor() -> Vec<u8> {
        vec![18, 0x01, 0x00, 0x02, 0xEF, 0x02, 0x01, 0x40, 0x64, 0x35, 0xF9, 0xFE, 0x00, 0x01, 1, 2, 3, 1]
    }
    fn configuration() -> Vec<u8> {
        // wTotalLength isn't checked by the parser, so a placeholder is fine.
        vec![9, 0x02, 0x00, 0x01, 2, 1, 0, 0x80, 250]
    }
    fn interface(number: u8, class: u8, subclass: u8) -> Vec<u8> {
        vec![9, 0x04, number, 0, 1, class, subclass, 0, 0]
    }
    fn extension_unit(unit: u8, guid: &[u8; 16]) -> Vec<u8> {
        let mut d = vec![28, 0x24, 0x06, unit];
        d.extend_from_slice(guid);
        // 19 controls, one input pin (unit 1), a 3-byte control bitmap, no name.
        d.extend_from_slice(&[19, 1, 1, 3, 0xFF, 0xFF, 0x07, 0]);
        d
    }

    fn typical_camera() -> Vec<u8> {
        let mut bytes = device_descriptor();
        bytes.extend(configuration());
        bytes.extend([8, 0x0B, 0, 2, 0x0E, 0x03, 0, 0]); // interface association
        bytes.extend(interface(0, 0x0E, 0x01));
        bytes.extend([13, 0x24, 0x01, 0x00, 0x01, 0x60, 0, 0x00, 0x6C, 0xDC, 0x02, 1, 1]); // VC header
        bytes.extend([18, 0x24, 0x02, 1, 0x01, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0x0A, 0x08]); // camera terminal
        let mut other = GUID;
        other[0] ^= 0xFF;
        bytes.extend(extension_unit(4, &other));
        bytes.extend(extension_unit(3, &GUID));
        bytes.extend([7, 0x05, 0x83, 0x03, 0x10, 0x00, 0x08]); // interrupt endpoint
        bytes
    }

    #[test]
    fn finds_the_extension_unit() {
        assert_eq!(find_xu_unit(&typical_camera(), &GUID), Some(3));
    }

    #[test]
    fn ignores_other_guids_and_missing_units() {
        let mut other = GUID;
        other[15] = 0;
        assert_eq!(find_xu_unit(&typical_camera(), &other), None);
        assert_eq!(find_xu_unit(&[], &GUID), None);
        assert_eq!(find_xu_unit(&device_descriptor(), &GUID), None);
    }

    #[test]
    fn ignores_matching_bytes_outside_video_control() {
        // Subtype 6 in a VideoStreaming interface is an MJPEG format descriptor.
        let mut bytes = device_descriptor();
        bytes.extend(configuration());
        bytes.extend(interface(1, 0x0E, 0x02));
        bytes.extend(extension_unit(5, &GUID));
        assert_eq!(find_xu_unit(&bytes, &GUID), None);
        // Nor before any interface at all.
        let mut bytes = device_descriptor();
        bytes.extend(extension_unit(5, &GUID));
        assert_eq!(find_xu_unit(&bytes, &GUID), None);
    }

    #[test]
    fn copes_with_damaged_descriptors() {
        let good = typical_camera();
        // Cut off in the middle of our unit: not found, no panic.
        let cut = good.len() - 7 - 10;
        assert_eq!(find_xu_unit(&good[..cut], &GUID), None);
        // Every truncation is handled.
        for n in 0..good.len() {
            let _ = find_xu_unit(&good[..n], &GUID);
        }
        // A zero length stops the walk instead of looping forever.
        let mut zero = device_descriptor();
        zero.extend([0, 0x24, 0x06]);
        zero.extend(extension_unit(3, &GUID));
        assert_eq!(find_xu_unit(&zero, &GUID), None);
        // Too short to hold a GUID.
        let mut short = device_descriptor();
        short.extend(interface(0, 0x0E, 0x01));
        short.extend([4, 0x24, 0x06, 3]);
        assert_eq!(find_xu_unit(&short, &GUID), None);
    }

    #[test]
    fn auto_pairs() {
        assert_eq!(control_ids(UvcCtl::Exposure).auto, Some(v4l2::CID_EXPOSURE_AUTO));
        assert_eq!(control_ids(UvcCtl::WhiteBalance).auto, Some(v4l2::CID_AUTO_WHITE_BALANCE));
        assert_eq!(control_ids(UvcCtl::Focus).auto, Some(v4l2::CID_FOCUS_AUTO));
        assert_eq!(control_ids(UvcCtl::Hue).auto, Some(v4l2::CID_HUE_AUTO));
        assert!(control_ids(UvcCtl::Zoom).auto.is_none());
        assert_eq!(manual_value(v4l2::CID_EXPOSURE_AUTO), v4l2::EXPOSURE_MANUAL);
        assert_eq!(manual_value(v4l2::CID_FOCUS_AUTO), 0);
        // Every control maps to a distinct V4L2 control.
        let mut ids: Vec<u32> = UvcCtl::ALL.iter().map(|&c| control_ids(c).value).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), UvcCtl::COUNT);
    }

    #[test]
    fn closed_camera_fails_cleanly() {
        let mut cam = LinuxCamera::new();
        assert!(!cam.is_open());
        assert!(cam.query_uvc(UvcCtl::Zoom).is_none());
        assert!(cam.get_uvc(UvcCtl::Zoom).is_none());
        assert!(cam.set_uvc(UvcCtl::Zoom, 1, false).is_err());
        assert!(cam.xu_get(SELECTOR_SIMPLE).is_err());
        assert!(cam.xu_set(SELECTOR_SIMPLE, &[0; XU_LENGTH]).is_err());
        let missing = DeviceInfo { path: "/nonexistent/video0".into(), ..Default::default() };
        assert!(!cam.open(&missing));
    }
}
