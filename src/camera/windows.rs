//! Windows camera backend: finds OBSBOT cameras with DirectShow, drives the standard
//! UVC controls through IAMCameraControl / IAMVideoProcAmp, and reaches the vendor
//! extension unit through IKsTopologyInfo + IKsControl. Everything runs on the
//! camera worker's thread, which joins the multithreaded COM apartment.

use windows::core::{Interface, BSTR, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_INVALID_DATA, E_INVALIDARG, E_NOINTERFACE, S_OK};
use windows::Win32::Media::DirectShow::{
    CameraControl_Exposure, CameraControl_Focus, CameraControl_Pan, CameraControl_Tilt, CameraControl_Zoom,
    IAMCameraControl, IAMVideoProcAmp, IBaseFilter, ICreateDevEnum, VideoProcAmp_BacklightCompensation,
    VideoProcAmp_Brightness, VideoProcAmp_Contrast, VideoProcAmp_Gain, VideoProcAmp_Gamma, VideoProcAmp_Hue,
    VideoProcAmp_Saturation, VideoProcAmp_Sharpness, VideoProcAmp_WhiteBalance,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsTopologyInfo, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSNODETYPE_DEV_SPECIFIC,
    KSPROPERTY_TYPE_GET, KSPROPERTY_TYPE_SET, KSPROPERTY_TYPE_TOPOLOGY, KSP_NODE,
};
use windows::Win32::Media::MediaFoundation::{CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IEnumMoniker, IMoniker, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_BSTR};

use super::{CameraBackend, DeviceInfo, UvcCtl, UvcInfo};
use crate::protocol::{self, Model, XuBuffer, XU_LENGTH};

/// OBSBOT vendor extension unit {9A1E7291-6843-4683-6D92-39BC7906EE49}.
const XU_GUID: GUID = GUID::from_u128(0x9a1e7291_6843_4683_6d92_39bc7906ee49);
/// KSPROPERTY_VIDEOPROCAMP_POWERLINE_FREQUENCY: not in the VideoProcAmpProperty list.
const PROC_AMP_POWER_LINE_FREQUENCY: i32 = 13;
/// CameraControl_Flags_Auto == VideoProcAmp_Flags_Auto, and likewise for _Manual.
const FLAG_AUTO: i32 = 0x1;
const FLAG_MANUAL: i32 = 0x2;

/// "0x80070005 (Access is denied.)": the code plus the system's message, preferring
/// New Zealand English and falling back to the default language.
pub(crate) fn hr_text(hr: HRESULT) -> String {
    let mut text = format!("0x{:08X}", hr.0 as u32);
    // MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_NZ)
    for language in [0x1409u32, 0] {
        if let Some(message) = system_message(hr.0 as u32, language) {
            text.push_str(&format!(" ({message})"));
            break;
        }
    }
    text
}

fn system_message(code: u32, language: u32) -> Option<String> {
    let mut buffer = PWSTR::null();
    let flags = FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    // SAFETY: with ALLOCATE_BUFFER, lpbuffer is really a pointer to our PWSTR, which
    // FormatMessageW fills with a LocalAlloc'd string of the returned length.
    let len = unsafe {
        FormatMessageW(
            flags,
            None,
            code,
            language,
            PWSTR(&mut buffer.0 as *mut _ as _),
            0,
            None,
        )
    };
    if buffer.is_null() {
        return None;
    }
    // SAFETY: FormatMessageW wrote `len` UTF-16 units at `buffer`.
    let wide = unsafe { std::slice::from_raw_parts(buffer.0, len as usize) };
    let message = String::from_utf16_lossy(wide);
    // SAFETY: the buffer was allocated by FormatMessageW with LocalAlloc.
    unsafe {
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(buffer.0 as _)));
    }
    let message = message.trim_end_matches(['\r', '\n', ' ']).to_string();
    (len > 0 && !message.is_empty()).then_some(message)
}

fn err_text(error: &windows::core::Error) -> String {
    hr_text(error.code())
}

/// Which DirectShow interface and property a control maps to.
fn map_uvc(control: UvcCtl) -> (bool, i32) {
    match control {
        UvcCtl::Pan => (true, CameraControl_Pan.0),
        UvcCtl::Tilt => (true, CameraControl_Tilt.0),
        UvcCtl::Zoom => (true, CameraControl_Zoom.0),
        UvcCtl::Exposure => (true, CameraControl_Exposure.0),
        UvcCtl::Focus => (true, CameraControl_Focus.0),
        UvcCtl::Brightness => (false, VideoProcAmp_Brightness.0),
        UvcCtl::Contrast => (false, VideoProcAmp_Contrast.0),
        UvcCtl::Hue => (false, VideoProcAmp_Hue.0),
        UvcCtl::Saturation => (false, VideoProcAmp_Saturation.0),
        UvcCtl::Sharpness => (false, VideoProcAmp_Sharpness.0),
        UvcCtl::Gamma => (false, VideoProcAmp_Gamma.0),
        UvcCtl::WhiteBalance => (false, VideoProcAmp_WhiteBalance.0),
        UvcCtl::BacklightComp => (false, VideoProcAmp_BacklightCompensation.0),
        UvcCtl::Gain => (false, VideoProcAmp_Gain.0),
        UvcCtl::PowerLine => (false, PROC_AMP_POWER_LINE_FREQUENCY),
    }
}

/// Finds "vid_XXXX" / "pid_XXXX" (any case) in a device path; 0 when absent.
fn parse_usb_id(path: &str, key: &str) -> u16 {
    let lower = path.to_ascii_lowercase();
    let Some(pos) = lower.find(key) else { return 0 };
    let Some(digits) = lower.get(pos + 4..pos + 8) else {
        return 0;
    };
    // Like wcstoul: read the leading hex digits and stop at the first other character.
    let hex: String = digits.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u16::from_str_radix(&hex, 16).unwrap_or(0)
}

fn read_bag_string(bag: &IPropertyBag, name: PCWSTR) -> String {
    let mut v = VARIANT::default();
    let mut out = String::new();
    // SAFETY: `v` is an initialised (empty) VARIANT that Read fills in; we only look at
    // bstrVal after checking the type tag, and VariantClear frees whatever Read put there.
    unsafe {
        if bag.Read(name, &mut v, None).is_ok() && v.Anonymous.Anonymous.vt == VT_BSTR {
            let b: &BSTR = &v.Anonymous.Anonymous.Anonymous.bstrVal;
            out = String::from_utf16_lossy(b);
        }
        let _ = VariantClear(&mut v);
    }
    out
}

/// Calls f(moniker, path, friendly name) for every video input device until f
/// returns false.
fn for_each_video_device(mut f: impl FnMut(&IMoniker, &str, &str) -> bool) {
    // SAFETY: plain COM creation on a COM-initialised thread.
    let dev_enum: ICreateDevEnum =
        match unsafe { CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER) } {
            Ok(e) => e,
            Err(e) => {
                crate::log_error!("Could not create the device enumerator: {}", err_text(&e));
                return;
            }
        };
    let mut monikers: Option<IEnumMoniker> = None;
    // SAFETY: the out pointer is a valid Option<IEnumMoniker>. S_FALSE (no video
    // devices at all) succeeds but leaves it empty.
    if unsafe { dev_enum.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut monikers, 0) }.is_err() {
        return;
    }
    let Some(monikers) = monikers else { return };
    loop {
        let mut slot = [None];
        // SAFETY: one slot for one moniker; fetched count isn't needed.
        if unsafe { monikers.Next(&mut slot, None) } != S_OK {
            break;
        }
        let Some(moniker) = slot[0].take() else { break };
        // SAFETY: binding a device moniker to its property bag, no bind context needed.
        let Ok(bag) = (unsafe { moniker.BindToStorage::<_, _, IPropertyBag>(None, None) }) else {
            continue;
        };
        let path = read_bag_string(&bag, windows::core::w!("DevicePath"));
        let name = read_bag_string(&bag, windows::core::w!("FriendlyName"));
        if !f(&moniker, &path, &name) {
            break;
        }
    }
}

pub struct WindowsCamera {
    com: bool,
    info: DeviceInfo,
    filter: Option<IBaseFilter>,
    camera_control: Option<IAMCameraControl>,
    proc_amp: Option<IAMVideoProcAmp>,
    ks_control: Option<IKsControl>,
    xu_found: bool,
    xu_node: u32,
}

impl WindowsCamera {
    pub fn new() -> Self {
        WindowsCamera {
            com: false,
            info: DeviceInfo::default(),
            filter: None,
            camera_control: None,
            proc_amp: None,
            ks_control: None,
            xu_found: false,
            xu_node: 0,
        }
    }

    fn find_xu_node(&mut self) -> bool {
        if self.ks_control.is_none() {
            return false;
        }
        let Some(topo) = self.filter.as_ref().and_then(|f| f.cast::<IKsTopologyInfo>().ok()) else {
            return false;
        };
        // SAFETY: simple getters on a live interface.
        let Ok(nodes) = (unsafe { topo.NumNodes() }) else {
            return false;
        };
        for i in 0..nodes {
            // SAFETY: i is below NumNodes.
            match unsafe { topo.get_NodeType(i) } {
                Ok(t) if t == KSNODETYPE_DEV_SPECIFIC => {}
                _ => continue,
            }
            // Probe: a harmless GET of the status block only succeeds on our XU.
            let mut probe = [0u8; XU_LENGTH];
            if self
                .xu_transfer(
                    protocol::SELECTOR_SIMPLE,
                    KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_TOPOLOGY,
                    i,
                    &mut probe,
                )
                .is_ok()
            {
                self.xu_node = i;
                return true;
            }
        }
        false
    }

    /// One KS property request on the extension unit. A GET must fill the whole buffer.
    fn xu_transfer(&self, selector: u8, flags: u32, node: u32, data: &mut XuBuffer) -> Result<(), HRESULT> {
        let Some(ks) = self.ks_control.as_ref() else {
            return Err(E_NOINTERFACE);
        };
        let prop = KSP_NODE {
            Property: KSIDENTIFIER {
                Anonymous: KSIDENTIFIER_0 {
                    Anonymous: KSIDENTIFIER_0_0 {
                        Set: XU_GUID,
                        Id: selector as u32,
                        Flags: flags,
                    },
                },
            },
            NodeId: node,
            Reserved: 0,
        };
        let mut returned = 0u32;
        // SAFETY: prop is a KSP_NODE (which starts with a KSIDENTIFIER) of the stated
        // size, and data is a writable buffer of exactly the stated length.
        let result = unsafe {
            ks.KsProperty(
                &prop as *const KSP_NODE as *const KSIDENTIFIER,
                std::mem::size_of::<KSP_NODE>() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut returned,
            )
        };
        result.map_err(|e| e.code())?;
        if flags & KSPROPERTY_TYPE_GET != 0 && returned as usize != data.len() {
            return Err(HRESULT::from_win32(ERROR_INVALID_DATA.0));
        }
        Ok(())
    }

    fn range(&self, control: UvcCtl) -> Option<(i32, i32, i32, i32, i32)> {
        let (is_camera, property) = map_uvc(control);
        let (mut min, mut max, mut step, mut def, mut caps) = (0, 0, 0, 0, 0);
        // SAFETY: all out pointers are valid locals.
        let result = unsafe {
            match (is_camera, &self.camera_control, &self.proc_amp) {
                (true, Some(cc), _) => cc.GetRange(property, &mut min, &mut max, &mut step, &mut def, &mut caps),
                (false, _, Some(pa)) => pa.GetRange(property, &mut min, &mut max, &mut step, &mut def, &mut caps),
                _ => return None,
            }
        };
        (result.is_ok() && max >= min).then_some((min, max, step, def, caps))
    }
}

impl Default for WindowsCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WindowsCamera {
    fn drop(&mut self) {
        self.close();
    }
}

impl CameraBackend for WindowsCamera {
    fn thread_start(&mut self) -> Result<(), String> {
        // SAFETY: called once on the worker thread; balanced by thread_end.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_err() {
            return Err(hr_text(hr));
        }
        self.com = true;
        Ok(())
    }

    fn thread_end(&mut self) {
        // Every COM pointer must go before the apartment does.
        self.close();
        if self.com {
            // SAFETY: balances the successful CoInitializeEx in thread_start, same thread.
            unsafe { CoUninitialize() };
            self.com = false;
        }
    }

    fn enumerate(&mut self) -> Vec<DeviceInfo> {
        let mut out = Vec::new();
        for_each_video_device(|_, path, name| {
            let vid = parse_usb_id(path, "vid_");
            if vid != protocol::VENDOR_ID {
                return true;
            }
            let pid = parse_usb_id(path, "pid_");
            out.push(DeviceInfo {
                path: path.to_string(),
                name: name.to_string(),
                vid,
                pid,
                model: Model::from_pid(pid),
            });
            true
        });
        out
    }

    fn open(&mut self, info: &DeviceInfo) -> bool {
        self.close();
        let mut filter = None;
        for_each_video_device(|moniker, path, _| {
            if !path.eq_ignore_ascii_case(&info.path) {
                return true;
            }
            // SAFETY: binding a device moniker to its filter, no bind context needed.
            match unsafe { moniker.BindToObject::<_, _, IBaseFilter>(None, None) } {
                Ok(f) => filter = Some(f),
                Err(e) => crate::log_error!("Could not open the camera: {}", err_text(&e)),
            }
            false
        });
        let Some(filter) = filter else { return false };
        self.info = info.clone();
        self.camera_control = filter.cast().ok();
        self.proc_amp = filter.cast().ok();
        self.ks_control = filter.cast().ok();
        self.filter = Some(filter);
        if self.camera_control.is_none() {
            crate::log_warn!("Camera has no IAMCameraControl (pan/tilt/zoom unavailable).");
        }
        if self.proc_amp.is_none() {
            crate::log_warn!("Camera has no IAMVideoProcAmp (image controls unavailable).");
        }
        self.xu_found = self.find_xu_node();
        if self.xu_found {
            crate::log_info!("Found the OBSBOT extension unit on KS node {}.", self.xu_node);
        } else {
            crate::log_warn!(
                "OBSBOT extension unit not found: AI tracking, HDR, FOV and sleep controls are unavailable."
            );
        }
        true
    }

    fn close(&mut self) {
        self.ks_control = None;
        self.proc_amp = None;
        self.camera_control = None;
        self.filter = None;
        self.xu_found = false;
        self.xu_node = 0;
    }

    fn is_open(&self) -> bool {
        self.filter.is_some()
    }

    fn has_xu(&self) -> bool {
        self.xu_found
    }

    fn xu_location(&self) -> u32 {
        self.xu_node
    }

    fn query_uvc(&mut self, control: UvcCtl) -> Option<UvcInfo> {
        let (min, max, step, def, caps) = self.range(control)?;
        let (value, is_auto) = self.get_uvc(control).unwrap_or((0, false));
        Some(UvcInfo {
            supported: true,
            min,
            max,
            step: if step > 0 { step } else { 1 },
            def,
            can_auto: caps & FLAG_AUTO != 0,
            can_manual: caps & FLAG_MANUAL != 0 || caps == 0,
            value,
            is_auto,
        })
    }

    fn get_uvc(&mut self, control: UvcCtl) -> Option<(i32, bool)> {
        let (is_camera, property) = map_uvc(control);
        let (mut value, mut flags) = (0, 0);
        // SAFETY: both out pointers are valid locals.
        let result = unsafe {
            match (is_camera, &self.camera_control, &self.proc_amp) {
                (true, Some(cc), _) => cc.Get(property, &mut value, &mut flags),
                (false, _, Some(pa)) => pa.Get(property, &mut value, &mut flags),
                _ => return None,
            }
        };
        result.ok().map(|_| (value, flags & FLAG_AUTO != 0))
    }

    fn set_uvc(&mut self, control: UvcCtl, value: i32, auto: bool) -> Result<(), String> {
        let Some(range) = self.query_uvc(control) else {
            return Err(hr_text(E_NOINTERFACE));
        };
        if (auto && !range.can_auto) || (!auto && !range.can_manual) {
            return Err(hr_text(E_INVALIDARG));
        }
        let value = value.clamp(range.min, range.max);
        let (is_camera, property) = map_uvc(control);
        let flags = if auto { FLAG_AUTO } else { FLAG_MANUAL };
        // SAFETY: plain value call on a live interface.
        let result = unsafe {
            match (is_camera, &self.camera_control, &self.proc_amp) {
                (true, Some(cc), _) => cc.Set(property, value, flags),
                (false, _, Some(pa)) => pa.Set(property, value, flags),
                _ => return Err(hr_text(E_NOINTERFACE)),
            }
        };
        result.map_err(|e| err_text(&e))
    }

    fn xu_set(&mut self, selector: u8, data: &XuBuffer) -> Result<(), String> {
        if !self.xu_found {
            return Err(hr_text(E_NOINTERFACE));
        }
        let mut copy = *data;
        self.xu_transfer(
            selector,
            KSPROPERTY_TYPE_SET | KSPROPERTY_TYPE_TOPOLOGY,
            self.xu_node,
            &mut copy,
        )
        .map_err(hr_text)
    }

    fn xu_get(&mut self, selector: u8) -> Result<XuBuffer, String> {
        if !self.xu_found {
            return Err(hr_text(E_NOINTERFACE));
        }
        let mut data = [0u8; XU_LENGTH];
        self.xu_transfer(
            selector,
            KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_TOPOLOGY,
            self.xu_node,
            &mut data,
        )
        .map_err(hr_text)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_usb_id;

    #[test]
    fn usb_ids_from_device_path() {
        let path = r"\\?\USB#VID_3564&PID_FEF9&MI_00#6&1a2b3c&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        assert_eq!(parse_usb_id(path, "vid_"), 0x3564);
        assert_eq!(parse_usb_id(path, "pid_"), 0xFEF9);
        assert_eq!(parse_usb_id("usb#vid_35", "vid_"), 0);
        assert_eq!(parse_usb_id("no ids here", "pid_"), 0);
    }
}
