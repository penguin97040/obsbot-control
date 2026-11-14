//! The Linux video driver interface (V4L2) and the UVC driver's extension-unit
//! ioctl: kernel structs, constants and ioctl numbers, plus small helpers shared by
//! the camera, preview and camera-use code. Layouts follow <linux/videodev2.h> and
//! <linux/uvcvideo.h>; the tests check them against the kernel headers.

use std::ffi::{c_void, CString};
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

// ---- ioctl numbers ---------------------------------------------------------------

// The generic Linux encoding (x86, Arm, RISC-V): direction in the top two bits,
// then a 14-bit argument size, the type letter and the command number.
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;
const IOC_SIZE_MASK: u32 = 0x3FFF;

const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> libc::Ioctl {
    ((dir << 30) | (((size as u32) & IOC_SIZE_MASK) << 16) | ((kind as u32) << 8) | nr as u32) as libc::Ioctl
}
const fn ior<T>(kind: u8, nr: u8) -> libc::Ioctl {
    ioc(IOC_READ, kind, nr, size_of::<T>())
}
const fn iow<T>(kind: u8, nr: u8) -> libc::Ioctl {
    ioc(IOC_WRITE, kind, nr, size_of::<T>())
}
const fn iowr<T>(kind: u8, nr: u8) -> libc::Ioctl {
    ioc(IOC_READ | IOC_WRITE, kind, nr, size_of::<T>())
}

pub const VIDIOC_QUERYCAP: libc::Ioctl = ior::<Capability>(b'V', 0);
pub const VIDIOC_ENUM_FMT: libc::Ioctl = iowr::<FmtDesc>(b'V', 2);
pub const VIDIOC_S_FMT: libc::Ioctl = iowr::<Format>(b'V', 5);
pub const VIDIOC_REQBUFS: libc::Ioctl = iowr::<RequestBuffers>(b'V', 8);
pub const VIDIOC_QUERYBUF: libc::Ioctl = iowr::<Buffer>(b'V', 9);
pub const VIDIOC_QBUF: libc::Ioctl = iowr::<Buffer>(b'V', 15);
pub const VIDIOC_DQBUF: libc::Ioctl = iowr::<Buffer>(b'V', 17);
pub const VIDIOC_STREAMON: libc::Ioctl = iow::<i32>(b'V', 18);
pub const VIDIOC_STREAMOFF: libc::Ioctl = iow::<i32>(b'V', 19);
pub const VIDIOC_S_PARM: libc::Ioctl = iowr::<StreamParm>(b'V', 22);
pub const VIDIOC_G_CTRL: libc::Ioctl = iowr::<Control>(b'V', 27);
pub const VIDIOC_S_CTRL: libc::Ioctl = iowr::<Control>(b'V', 28);
pub const VIDIOC_QUERYCTRL: libc::Ioctl = iowr::<QueryCtrl>(b'V', 36);
pub const VIDIOC_ENUM_FRAMESIZES: libc::Ioctl = iowr::<FrmSizeEnum>(b'V', 74);
pub const VIDIOC_ENUM_FRAMEINTERVALS: libc::Ioctl = iowr::<FrmIvalEnum>(b'V', 75);
const UVCIOC_CTRL_QUERY: libc::Ioctl = iowr::<XuControlQuery>(b'u', 0x21);

// ---- Constants -------------------------------------------------------------------

pub const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const FIELD_NONE: u32 = 1;
pub const BUF_FLAG_ERROR: u32 = 0x0000_0040;
pub const FMT_FLAG_EMULATED: u32 = 0x0000_0002;

pub const FRMSIZE_TYPE_DISCRETE: u32 = 1;
pub const FRMIVAL_TYPE_DISCRETE: u32 = 1;

pub const CTRL_FLAG_DISABLED: u32 = 0x0001;
pub const CTRL_FLAG_READ_ONLY: u32 = 0x0004;

/// Builds a pixel format code from its four characters.
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}
pub const PIX_FMT_MJPEG: u32 = fourcc(b"MJPG");
pub const PIX_FMT_YUYV: u32 = fourcc(b"YUYV");
pub const PIX_FMT_NV12: u32 = fourcc(b"NV12");

// Control IDs: the user class starts at 0x00980900, the camera class at 0x009A0900.
const CID_BASE: u32 = 0x0098_0900;
const CID_CAMERA_BASE: u32 = 0x009A_0900;
pub const CID_BRIGHTNESS: u32 = CID_BASE;
pub const CID_CONTRAST: u32 = CID_BASE + 1;
pub const CID_SATURATION: u32 = CID_BASE + 2;
pub const CID_HUE: u32 = CID_BASE + 3;
pub const CID_AUTO_WHITE_BALANCE: u32 = CID_BASE + 12;
pub const CID_GAMMA: u32 = CID_BASE + 16;
pub const CID_GAIN: u32 = CID_BASE + 19;
pub const CID_POWER_LINE_FREQUENCY: u32 = CID_BASE + 24;
pub const CID_HUE_AUTO: u32 = CID_BASE + 25;
pub const CID_WHITE_BALANCE_TEMPERATURE: u32 = CID_BASE + 26;
pub const CID_SHARPNESS: u32 = CID_BASE + 27;
pub const CID_BACKLIGHT_COMPENSATION: u32 = CID_BASE + 28;
pub const CID_EXPOSURE_AUTO: u32 = CID_CAMERA_BASE + 1;
pub const CID_EXPOSURE_ABSOLUTE: u32 = CID_CAMERA_BASE + 2;
pub const CID_PAN_ABSOLUTE: u32 = CID_CAMERA_BASE + 8;
pub const CID_TILT_ABSOLUTE: u32 = CID_CAMERA_BASE + 9;
pub const CID_FOCUS_ABSOLUTE: u32 = CID_CAMERA_BASE + 10;
pub const CID_FOCUS_AUTO: u32 = CID_CAMERA_BASE + 12;
pub const CID_ZOOM_ABSOLUTE: u32 = CID_CAMERA_BASE + 13;

// Menu items of CID_EXPOSURE_AUTO.
pub const EXPOSURE_AUTO: i32 = 0;
pub const EXPOSURE_MANUAL: i32 = 1;
pub const EXPOSURE_SHUTTER_PRIORITY: i32 = 2;
pub const EXPOSURE_APERTURE_PRIORITY: i32 = 3;

// UVC request codes for the extension unit.
pub const UVC_SET_CUR: u8 = 0x01;
pub const UVC_GET_CUR: u8 = 0x81;

// ---- Structs ---------------------------------------------------------------------

/// Kernel structs that are valid when all zero, and that carry no pointer the
/// kernel follows for the requests this app makes. Only these go through `ioctl`.
///
/// # Safety
/// Implementors must be `repr(C)` (or a primitive) with every bit pattern of zero
/// bytes valid.
pub unsafe trait Plain: Sized {}

/// A zero-filled kernel struct, ready to fill in.
pub fn zeroed<T: Plain>() -> T {
    // SAFETY: Plain promises that all-zero bytes are a valid T.
    unsafe { std::mem::zeroed() }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct QueryCtrl {
    pub id: u32,
    pub kind: u32,
    pub name: [u8; 32],
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
    pub flags: u32,
    pub reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Control {
    pub id: u32,
    pub value: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FmtDesc {
    pub index: u32,
    pub kind: u32,
    pub flags: u32,
    pub description: [u8; 32],
    pub pixelformat: u32,
    pub mbus_code: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrmSizeDiscrete {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrmSizeStepwise {
    pub min_width: u32,
    pub max_width: u32,
    pub step_width: u32,
    pub min_height: u32,
    pub max_height: u32,
    pub step_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union FrmSizeUnion {
    pub discrete: FrmSizeDiscrete,
    pub stepwise: FrmSizeStepwise,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrmSizeEnum {
    pub index: u32,
    pub pixel_format: u32,
    pub kind: u32,
    pub size: FrmSizeUnion,
    pub reserved: [u32; 2],
}

/// A fraction; for frame intervals it is seconds per frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fract {
    pub numerator: u32,
    pub denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrmIvalStepwise {
    pub min: Fract,
    pub max: Fract,
    pub step: Fract,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union FrmIvalUnion {
    pub discrete: Fract,
    pub stepwise: FrmIvalStepwise,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrmIvalEnum {
    pub index: u32,
    pub pixel_format: u32,
    pub width: u32,
    pub height: u32,
    pub kind: u32,
    pub interval: FrmIvalUnion,
    pub reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PixFormat {
    pub width: u32,
    pub height: u32,
    pub pixelformat: u32,
    pub field: u32,
    pub bytesperline: u32,
    pub sizeimage: u32,
    pub colorspace: u32,
    pub private: u32,
    pub flags: u32,
    pub ycbcr_enc: u32,
    pub quantization: u32,
    pub xfer_func: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union FormatUnion {
    pub pix: PixFormat,
    pub raw_data: [u8; 200],
    // The kernel's union holds structs with pointers, which sets its alignment.
    _align: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Format {
    pub kind: u32,
    pub fmt: FormatUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CaptureParm {
    pub capability: u32,
    pub capturemode: u32,
    pub timeperframe: Fract,
    pub extendedmode: u32,
    pub readbuffers: u32,
    pub reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union StreamParmUnion {
    pub capture: CaptureParm,
    pub raw_data: [u8; 200],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StreamParm {
    pub kind: u32,
    pub parm: StreamParmUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RequestBuffers {
    pub count: u32,
    pub kind: u32,
    pub memory: u32,
    pub capabilities: u32,
    pub flags: u8,
    pub reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timecode {
    pub kind: u32,
    pub flags: u32,
    pub frames: u8,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub userbits: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union BufferM {
    pub offset: u32,
    pub userptr: libc::c_ulong,
    pub planes: *mut c_void,
    pub fd: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Buffer {
    pub index: u32,
    pub kind: u32,
    pub bytesused: u32,
    pub flags: u32,
    pub field: u32,
    pub timestamp: libc::timeval,
    pub timecode: Timecode,
    pub sequence: u32,
    /// Always MEMORY_MMAP here, so the kernel never follows `m` as a pointer.
    pub memory: u32,
    pub m: BufferM,
    pub length: u32,
    pub reserved2: u32,
    pub request_fd: i32,
}

/// struct uvc_xu_control_query. Not Plain: the kernel follows `data`, so it is only
/// built inside `uvc_xu_query`.
#[repr(C)]
struct XuControlQuery {
    unit: u8,
    selector: u8,
    query: u8,
    size: u16,
    data: *mut u8,
}

// SAFETY: each is repr(C) plain data (integers, arrays, unions of those, and
// pointers that are valid when null), so all-zero bytes are valid. Buffer's
// pointer member is only meaningful for non-MMAP memory, which we never use.
unsafe impl Plain for Capability {}
unsafe impl Plain for QueryCtrl {}
unsafe impl Plain for Control {}
unsafe impl Plain for FmtDesc {}
unsafe impl Plain for FrmSizeEnum {}
unsafe impl Plain for FrmIvalEnum {}
unsafe impl Plain for PixFormat {}
unsafe impl Plain for Format {}
unsafe impl Plain for StreamParm {}
unsafe impl Plain for RequestBuffers {}
unsafe impl Plain for Buffer {}
unsafe impl Plain for i32 {}

// ---- Calls -----------------------------------------------------------------------

/// The calling thread's last OS error number.
pub fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Runs an ioctl whose argument is `arg`, retrying when a signal interrupts it.
/// Returns the error number on failure.
pub fn ioctl<T: Plain>(fd: &impl AsRawFd, request: libc::Ioctl, arg: &mut T) -> Result<(), i32> {
    // The request number encodes how many bytes the kernel copies in and out;
    // refuse a mismatch rather than let it read or write past `arg`.
    if ((request as u32 >> 16) & IOC_SIZE_MASK) as usize != size_of::<T>() {
        return Err(libc::EINVAL);
    }
    loop {
        // SAFETY: `arg` is a live, exclusively borrowed T of exactly the size the
        // request encodes, and Plain types hold no pointers the kernel follows.
        let r = unsafe { libc::ioctl(fd.as_raw_fd(), request, arg as *mut T as *mut c_void) };
        if r >= 0 {
            return Ok(());
        }
        let e = last_errno();
        if e != libc::EINTR {
            return Err(e);
        }
    }
}

/// One UVC extension-unit request (UVC_GET_CUR or UVC_SET_CUR) of `data.len()` bytes.
pub fn uvc_xu_query(fd: &impl AsRawFd, unit: u8, selector: u8, query: u8, data: &mut [u8]) -> Result<(), i32> {
    let size = u16::try_from(data.len()).map_err(|_| libc::EINVAL)?;
    let mut q = XuControlQuery { unit, selector, query, size, data: data.as_mut_ptr() };
    loop {
        // SAFETY: `q` is a valid uvc_xu_control_query whose `data` points to `size`
        // writable bytes that stay borrowed for the whole call.
        let r = unsafe { libc::ioctl(fd.as_raw_fd(), UVCIOC_CTRL_QUERY, &mut q as *mut XuControlQuery as *mut c_void) };
        if r >= 0 {
            return Ok(());
        }
        let e = last_errno();
        if e != libc::EINTR {
            return Err(e);
        }
    }
}

/// Opens a video node the way every part of the app does: read-write (controls
/// need it), non-blocking (so a stalled camera can't hang a thread) and not
/// inherited by child processes.
pub fn open_node(path: &str) -> Result<OwnedFd, i32> {
    let c_path = CString::new(path).map_err(|_| libc::EINVAL)?;
    loop {
        // SAFETY: `c_path` is a valid NUL-terminated string for the whole call.
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd >= 0 {
            // SAFETY: `fd` was just returned by open and nothing else owns it.
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let e = last_errno();
        if e != libc::EINTR {
            return Err(e);
        }
    }
}

// ---- Text ------------------------------------------------------------------------

fn errno_name(e: i32) -> Option<&'static str> {
    Some(match e {
        libc::EPERM => "EPERM",
        libc::ENOENT => "ENOENT",
        libc::EINTR => "EINTR",
        libc::EIO => "EIO",
        libc::ENXIO => "ENXIO",
        libc::EBADF => "EBADF",
        libc::EAGAIN => "EAGAIN",
        libc::ENOMEM => "ENOMEM",
        libc::EACCES => "EACCES",
        libc::EFAULT => "EFAULT",
        libc::EBUSY => "EBUSY",
        libc::ENODEV => "ENODEV",
        libc::EINVAL => "EINVAL",
        libc::EMFILE => "EMFILE",
        libc::ENOTTY => "ENOTTY",
        libc::ENOSPC => "ENOSPC",
        libc::EPIPE => "EPIPE",
        libc::ERANGE => "ERANGE",
        libc::ENOSYS => "ENOSYS",
        libc::ENODATA => "ENODATA",
        libc::ETIME => "ETIME",
        libc::EPROTO => "EPROTO",
        libc::EOVERFLOW => "EOVERFLOW",
        libc::EILSEQ => "EILSEQ",
        libc::EMSGSIZE => "EMSGSIZE",
        libc::EOPNOTSUPP => "EOPNOTSUPP",
        libc::ENOBUFS => "ENOBUFS",
        libc::ESHUTDOWN => "ESHUTDOWN",
        libc::ETIMEDOUT => "ETIMEDOUT",
        _ => return None,
    })
}

/// e.g. "EBUSY (Device or resource busy)".
pub fn errno_text(e: i32) -> String {
    let full = std::io::Error::from_raw_os_error(e).to_string();
    // The standard library adds " (os error N)"; the name already says which.
    let message = full.rsplit_once(" (os error").map_or(full.as_str(), |(m, _)| m);
    match errno_name(e) {
        Some(name) => format!("{name} ({message})"),
        None => format!("error {e} ({message})"),
    }
}

/// Plain advice for when opening a video node is refused.
pub fn permission_hint(path: &str) -> String {
    format!(
        "This app may need permission to use {path}. On most distributions, logged-in users already have it; \
         otherwise add yourself to the \"video\" group, then log out and back in."
    )
}

/// A fixed-size, NUL-padded kernel string as text.
pub fn c_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// The N in /dev/videoN (or /sys/class/video4linux/videoN).
pub fn node_number(path: &str) -> Option<u32> {
    let name = path.rsplit('/').next()?;
    name.strip_prefix("video")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values printed by a C program built against the kernel headers
    // (<linux/videodev2.h>, <linux/uvcvideo.h>) on x86-64 Linux.
    #[test]
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    fn struct_sizes_match_the_kernel() {
        assert_eq!(size_of::<Capability>(), 104);
        assert_eq!(size_of::<QueryCtrl>(), 68);
        assert_eq!(size_of::<Control>(), 8);
        assert_eq!(size_of::<FmtDesc>(), 64);
        assert_eq!(size_of::<FrmSizeEnum>(), 44);
        assert_eq!(size_of::<FrmIvalEnum>(), 52);
        assert_eq!(size_of::<PixFormat>(), 48);
        assert_eq!(size_of::<Format>(), 208);
        assert_eq!(size_of::<CaptureParm>(), 40);
        assert_eq!(size_of::<StreamParm>(), 204);
        assert_eq!(size_of::<RequestBuffers>(), 20);
        assert_eq!(size_of::<Timecode>(), 16);
        assert_eq!(size_of::<Buffer>(), 88);
        assert_eq!(size_of::<XuControlQuery>(), 16);
    }

    #[test]
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    fn field_offsets_match_the_kernel() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(Format, fmt), 8);
        assert_eq!(offset_of!(Buffer, timestamp), 24);
        assert_eq!(offset_of!(Buffer, sequence), 56);
        assert_eq!(offset_of!(Buffer, m), 64);
        assert_eq!(offset_of!(Buffer, length), 72);
        assert_eq!(offset_of!(StreamParm, parm), 4);
        assert_eq!(offset_of!(XuControlQuery, size), 4);
        assert_eq!(offset_of!(XuControlQuery, data), 8);
        assert_eq!(offset_of!(FrmSizeEnum, size), 12);
        assert_eq!(offset_of!(FrmIvalEnum, interval), 20);
        assert_eq!(offset_of!(QueryCtrl, flags), 56);
    }

    #[test]
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    fn ioctl_numbers_match_the_kernel() {
        let n = |r: libc::Ioctl| r as u32;
        assert_eq!(n(VIDIOC_QUERYCAP), 0x8068_5600);
        assert_eq!(n(VIDIOC_ENUM_FMT), 0xC040_5602);
        assert_eq!(n(VIDIOC_S_FMT), 0xC0D0_5605);
        assert_eq!(n(VIDIOC_REQBUFS), 0xC014_5608);
        assert_eq!(n(VIDIOC_QUERYBUF), 0xC058_5609);
        assert_eq!(n(VIDIOC_QBUF), 0xC058_560F);
        assert_eq!(n(VIDIOC_DQBUF), 0xC058_5611);
        assert_eq!(n(VIDIOC_STREAMON), 0x4004_5612);
        assert_eq!(n(VIDIOC_STREAMOFF), 0x4004_5613);
        assert_eq!(n(VIDIOC_S_PARM), 0xC0CC_5616);
        assert_eq!(n(VIDIOC_G_CTRL), 0xC008_561B);
        assert_eq!(n(VIDIOC_S_CTRL), 0xC008_561C);
        assert_eq!(n(VIDIOC_QUERYCTRL), 0xC044_5624);
        assert_eq!(n(VIDIOC_ENUM_FRAMESIZES), 0xC02C_564A);
        assert_eq!(n(VIDIOC_ENUM_FRAMEINTERVALS), 0xC034_564B);
        assert_eq!(n(UVCIOC_CTRL_QUERY), 0xC010_7521);
    }

    #[test]
    fn constants_match_the_kernel() {
        assert_eq!(CID_BRIGHTNESS, 0x0098_0900);
        assert_eq!(CID_CONTRAST, 0x0098_0901);
        assert_eq!(CID_SATURATION, 0x0098_0902);
        assert_eq!(CID_HUE, 0x0098_0903);
        assert_eq!(CID_AUTO_WHITE_BALANCE, 0x0098_090C);
        assert_eq!(CID_GAMMA, 0x0098_0910);
        assert_eq!(CID_GAIN, 0x0098_0913);
        assert_eq!(CID_POWER_LINE_FREQUENCY, 0x0098_0918);
        assert_eq!(CID_HUE_AUTO, 0x0098_0919);
        assert_eq!(CID_WHITE_BALANCE_TEMPERATURE, 0x0098_091A);
        assert_eq!(CID_SHARPNESS, 0x0098_091B);
        assert_eq!(CID_BACKLIGHT_COMPENSATION, 0x0098_091C);
        assert_eq!(CID_EXPOSURE_AUTO, 0x009A_0901);
        assert_eq!(CID_EXPOSURE_ABSOLUTE, 0x009A_0902);
        assert_eq!(CID_PAN_ABSOLUTE, 0x009A_0908);
        assert_eq!(CID_TILT_ABSOLUTE, 0x009A_0909);
        assert_eq!(CID_FOCUS_ABSOLUTE, 0x009A_090A);
        assert_eq!(CID_FOCUS_AUTO, 0x009A_090C);
        assert_eq!(CID_ZOOM_ABSOLUTE, 0x009A_090D);
        assert_eq!(PIX_FMT_MJPEG, 0x4750_4A4D);
        assert_eq!(PIX_FMT_YUYV, 0x5659_5559);
        assert_eq!(PIX_FMT_NV12, 0x3231_564E);
        assert_eq!((EXPOSURE_AUTO, EXPOSURE_MANUAL), (0, 1));
        assert_eq!((EXPOSURE_SHUTTER_PRIORITY, EXPOSURE_APERTURE_PRIORITY), (2, 3));
    }

    #[test]
    fn ioctl_refuses_a_size_mismatch() {
        // A Control is 8 bytes; QUERYCTRL expects 68. No real call is made.
        let file = std::fs::File::open("/dev/null").unwrap();
        let mut c: Control = zeroed();
        assert_eq!(ioctl(&file, VIDIOC_QUERYCTRL, &mut c), Err(libc::EINVAL));
        // A real request on a file that isn't a video node fails cleanly.
        let mut q: QueryCtrl = zeroed();
        assert_eq!(ioctl(&file, VIDIOC_QUERYCTRL, &mut q), Err(libc::ENOTTY));
    }

    #[test]
    fn errno_text_is_readable() {
        assert_eq!(errno_text(libc::EBUSY), "EBUSY (Device or resource busy)");
        assert_eq!(errno_text(libc::EACCES), "EACCES (Permission denied)");
        assert!(errno_text(9999).starts_with("error 9999 ("));
    }

    #[test]
    fn helpers() {
        assert_eq!(node_number("/dev/video12"), Some(12));
        assert_eq!(node_number("/sys/class/video4linux/video0"), Some(0));
        assert_eq!(node_number("/dev/media0"), None);
        assert_eq!(c_text(b"uvcvideo\0\0\0junk"), "uvcvideo");
        assert_eq!(c_text(b"no nul"), "no nul");
        assert_eq!(open_node("/nonexistent/video0").err(), Some(libc::ENOENT));
    }
}
