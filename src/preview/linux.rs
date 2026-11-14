//! Linux preview: V4L2 memory-mapped streaming, converted to BGRA.

use std::os::fd::{AsRawFd, OwnedFd};

use super::convert;
use super::Sink;
use crate::preview_format::{self, PreviewFormat};
use crate::safety;
use crate::v4l2;

/// How many buffers to ask the driver for: enough to keep the camera busy while
/// one frame is being converted.
const BUFFER_COUNT: u32 = 4;
/// Upper bound on each poll, so a stop request is noticed quickly.
const POLL_MS: i32 = 100;
/// Guards against a driver that lists sizes or rates without end.
const MAX_ENUM: u32 = 256;

/// A readable reason the preview stopped, for the interface.
fn friendly_error(e: i32, path: &str) -> String {
    match e {
        libc::EBUSY => "The camera is busy. Another app (Zoom, OBS, a web browser...) may be using it.".into(),
        libc::ENODEV | libc::EIO | libc::ENOENT => "The camera was disconnected.".into(),
        libc::EACCES | libc::EPERM => v4l2::permission_hint(path),
        _ => format!("Preview failed: {}", v4l2::errno_text(e)),
    }
}

/// The label we show for a pixel format we can convert, or None to leave it out
/// (H.264 and anything else we can't decode).
fn fourcc_label(pixelformat: u32) -> Option<&'static str> {
    match pixelformat {
        v4l2::PIX_FMT_MJPEG => Some("MJPG"),
        v4l2::PIX_FMT_YUYV => Some("YUYV"),
        v4l2::PIX_FMT_NV12 => Some("NV12"),
        _ => None,
    }
}

fn pixelformat_for(label: &str) -> Option<u32> {
    [v4l2::PIX_FMT_MJPEG, v4l2::PIX_FMT_YUYV, v4l2::PIX_FMT_NV12]
        .into_iter()
        .find(|&f| fourcc_label(f) == Some(label))
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// A frame rate (frames per second) from a frame interval (seconds per frame),
/// reduced so keys stay short.
fn rate_from_interval(interval: v4l2::Fract) -> Option<(u32, u32)> {
    let (num, den) = (interval.numerator, interval.denominator);
    if num == 0 || den == 0 {
        return None;
    }
    let g = gcd(num, den);
    Some((den / g, num / g))
}

/// Common sizes that fit a stepwise or continuous range, plus its largest size.
fn stepwise_sizes(s: &v4l2::FrmSizeStepwise) -> Vec<(u32, u32)> {
    const COMMON: [(u32, u32); 10] = [
        (3840, 2160),
        (2560, 1440),
        (1920, 1080),
        (1600, 900),
        (1280, 720),
        (1024, 576),
        (960, 540),
        (800, 600),
        (640, 480),
        (640, 360),
    ];
    let fits = |value: u32, min: u32, max: u32, step: u32| {
        value >= min && value <= max && (step <= 1 || (value - min) % step == 0)
    };
    let mut sizes = vec![(s.max_width, s.max_height)];
    for (w, h) in COMMON {
        let wanted = fits(w, s.min_width, s.max_width, s.step_width) && fits(h, s.min_height, s.max_height, s.step_height);
        if wanted && !sizes.contains(&(w, h)) {
            sizes.push((w, h));
        }
    }
    sizes
}

/// Common rates inside a stepwise or continuous interval range.
fn stepwise_rates(s: &v4l2::FrmIvalStepwise) -> Vec<(u32, u32)> {
    let (min, max) = (s.min, s.max); // shortest and longest interval
    if min.numerator == 0 || min.denominator == 0 || max.denominator == 0 {
        return Vec::new();
    }
    // 1/fps lies in [min, max] when min.n * fps <= min.d and max.n * fps >= max.d.
    [60u64, 30, 25, 24, 15, 10, 5]
        .into_iter()
        .filter(|&fps| {
            min.numerator as u64 * fps <= min.denominator as u64 && max.numerator as u64 * fps >= max.denominator as u64
        })
        .map(|fps| (fps as u32, 1))
        .collect()
}

fn frame_sizes(fd: &OwnedFd, pixelformat: u32) -> Vec<(u32, u32)> {
    let mut sizes = Vec::new();
    for index in 0..MAX_ENUM {
        let mut e: v4l2::FrmSizeEnum = v4l2::zeroed();
        e.index = index;
        e.pixel_format = pixelformat;
        if v4l2::ioctl(fd, v4l2::VIDIOC_ENUM_FRAMESIZES, &mut e).is_err() {
            break;
        }
        if e.kind == v4l2::FRMSIZE_TYPE_DISCRETE {
            // SAFETY: the driver filled the discrete member, as `kind` says.
            let d = unsafe { e.size.discrete };
            sizes.push((d.width, d.height));
        } else {
            // A stepwise or continuous range is the only entry.
            // SAFETY: the driver filled the stepwise member, as `kind` says.
            let s = unsafe { e.size.stepwise };
            sizes.extend(stepwise_sizes(&s));
            break;
        }
    }
    sizes
}

fn frame_rates(fd: &OwnedFd, pixelformat: u32, width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut rates = Vec::new();
    for index in 0..MAX_ENUM {
        let mut e: v4l2::FrmIvalEnum = v4l2::zeroed();
        e.index = index;
        e.pixel_format = pixelformat;
        e.width = width;
        e.height = height;
        if v4l2::ioctl(fd, v4l2::VIDIOC_ENUM_FRAMEINTERVALS, &mut e).is_err() {
            break;
        }
        if e.kind == v4l2::FRMIVAL_TYPE_DISCRETE {
            // SAFETY: the driver filled the discrete member, as `kind` says.
            rates.extend(rate_from_interval(unsafe { e.interval.discrete }));
        } else {
            // SAFETY: the driver filled the stepwise member, as `kind` says.
            rates.extend(stepwise_rates(&unsafe { e.interval.stepwise }));
            break;
        }
    }
    rates
}

/// Every format the camera offers that we can show, in the driver's order.
fn list_formats(fd: &OwnedFd) -> Vec<PreviewFormat> {
    let mut list: Vec<PreviewFormat> = Vec::new();
    for index in 0..MAX_ENUM {
        let mut desc: v4l2::FmtDesc = v4l2::zeroed();
        desc.index = index;
        desc.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
        if v4l2::ioctl(fd, v4l2::VIDIOC_ENUM_FMT, &mut desc).is_err() {
            break;
        }
        let Some(label) = fourcc_label(desc.pixelformat) else {
            continue;
        };
        if desc.flags & v4l2::FMT_FLAG_EMULATED != 0 {
            continue;
        }
        for (width, height) in frame_sizes(fd, desc.pixelformat) {
            if !safety::geometry(width, height) {
                continue;
            }
            for (fps_num, fps_den) in frame_rates(fd, desc.pixelformat, width, height) {
                let f = PreviewFormat { width, height, fps_num, fps_den, fourcc: label.to_string() };
                if !list.contains(&f) {
                    list.push(f);
                }
            }
        }
    }
    list
}

/// One driver buffer mapped into our memory. Unmapped when dropped.
struct Mapping {
    ptr: *mut libc::c_void,
    len: usize,
}

impl Mapping {
    fn new(fd: &OwnedFd, offset: u32, len: usize) -> Result<Mapping, i32> {
        // SAFETY: mapping a driver buffer read-only, shared, at the offset and
        // length QUERYBUF reported; the kernel checks both.
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), offset as libc::off_t)
        };
        if ptr == libc::MAP_FAILED {
            return Err(v4l2::last_errno());
        }
        Ok(Mapping { ptr, len })
    }

    /// The first `used` bytes (never more than the mapping holds).
    ///
    /// Only call while the buffer is dequeued, so the driver isn't writing to it.
    fn bytes(&self, used: usize) -> &[u8] {
        // SAFETY: `ptr` is a live read-only mapping of `len` bytes, and we clamp to
        // it. While the buffer is dequeued the driver leaves it alone.
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, used.min(self.len)) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `ptr` and `len` came from a successful mmap that nothing else unmaps.
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

/// An open, streaming camera. Dropping it stops the stream, unmaps the buffers
/// and closes the node, in that order.
struct Stream {
    maps: Vec<Mapping>,
    fd: OwnedFd,
    streaming: bool,
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.streaming {
            let mut kind = v4l2::BUF_TYPE_VIDEO_CAPTURE as i32;
            let _ = v4l2::ioctl(&self.fd, v4l2::VIDIOC_STREAMOFF, &mut kind);
        }
        // Fields drop next: `maps` (munmap) before `fd` (close).
    }
}

fn new_buffer() -> v4l2::Buffer {
    let mut b: v4l2::Buffer = v4l2::zeroed();
    b.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
    b.memory = v4l2::MEMORY_MMAP;
    b
}

/// What the driver agreed to in S_FMT.
struct Negotiated {
    width: u32,
    height: u32,
    pixelformat: u32,
    bytesperline: usize,
}

fn set_format(fd: &OwnedFd, f: &PreviewFormat, pixelformat: u32) -> Result<Negotiated, i32> {
    let mut fmt: v4l2::Format = v4l2::zeroed();
    fmt.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
    let mut request: v4l2::PixFormat = v4l2::zeroed();
    request.width = f.width;
    request.height = f.height;
    request.pixelformat = pixelformat;
    request.field = v4l2::FIELD_NONE;
    fmt.fmt.pix = request;
    v4l2::ioctl(fd, v4l2::VIDIOC_S_FMT, &mut fmt)?;
    // SAFETY: for a capture type the driver replies in the pix member.
    let pix = unsafe { fmt.fmt.pix };
    Ok(Negotiated {
        width: pix.width,
        height: pix.height,
        pixelformat: pix.pixelformat,
        bytesperline: pix.bytesperline as usize,
    })
}

/// Asks for the frame rate. Returns the rate the driver chose, when it says.
fn set_frame_rate(fd: &OwnedFd, f: &PreviewFormat) -> Result<Option<(u32, u32)>, i32> {
    let mut parm: v4l2::StreamParm = v4l2::zeroed();
    parm.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
    // A rate of num/den frames per second is an interval of den/num seconds.
    let interval = v4l2::Fract { numerator: f.fps_den, denominator: f.fps_num };
    parm.parm.capture.timeperframe = interval;
    v4l2::ioctl(fd, v4l2::VIDIOC_S_PARM, &mut parm)?;
    // SAFETY: for a capture type the driver replies in the capture member.
    Ok(rate_from_interval(unsafe { parm.parm.capture.timeperframe }))
}

/// Requests, maps and queues the buffers, then starts streaming.
fn start_stream(fd: OwnedFd) -> Result<Stream, i32> {
    let mut stream = Stream { maps: Vec::new(), fd, streaming: false };
    let mut req: v4l2::RequestBuffers = v4l2::zeroed();
    req.count = BUFFER_COUNT;
    req.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
    req.memory = v4l2::MEMORY_MMAP;
    v4l2::ioctl(&stream.fd, v4l2::VIDIOC_REQBUFS, &mut req)?;
    if req.count == 0 {
        return Err(libc::ENOMEM);
    }
    for index in 0..req.count {
        let mut buf = new_buffer();
        buf.index = index;
        v4l2::ioctl(&stream.fd, v4l2::VIDIOC_QUERYBUF, &mut buf)?;
        // SAFETY: with MMAP memory the driver fills the offset member.
        let offset = unsafe { buf.m.offset };
        stream.maps.push(Mapping::new(&stream.fd, offset, buf.length as usize)?);
        v4l2::ioctl(&stream.fd, v4l2::VIDIOC_QBUF, &mut buf)?;
    }
    let mut kind = v4l2::BUF_TYPE_VIDEO_CAPTURE as i32;
    v4l2::ioctl(&stream.fd, v4l2::VIDIOC_STREAMON, &mut kind)?;
    stream.streaming = true;
    Ok(stream)
}

fn convert_frame(n: &Negotiated, data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    match n.pixelformat {
        v4l2::PIX_FMT_MJPEG => convert::mjpeg_to_bgra(data),
        v4l2::PIX_FMT_YUYV => {
            let stride = if n.bytesperline > 0 { n.bytesperline } else { n.width as usize * 2 };
            convert::yuyv_to_bgra(data, n.width, n.height, stride).map(|px| (px, n.width, n.height))
        }
        v4l2::PIX_FMT_NV12 => {
            let stride = if n.bytesperline > 0 { n.bytesperline } else { n.width as usize };
            convert::nv12_to_bgra(data, n.width, n.height, stride).map(|px| (px, n.width, n.height))
        }
        _ => None,
    }
}

/// Waits up to POLL_MS for a frame. Ok(false) means none arrived yet.
fn wait_for_frame(fd: &OwnedFd) -> Result<bool, i32> {
    let mut pfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: `pfd` is one valid pollfd for the whole call.
    let r = unsafe { libc::poll(&mut pfd, 1, POLL_MS) };
    if r < 0 {
        let e = v4l2::last_errno();
        return if e == libc::EINTR { Ok(false) } else { Err(e) };
    }
    // On an error or hang-up, DQBUF below reports the reason.
    Ok(r > 0)
}

pub fn run(device_path: &str, format_key: &str, sink: &Sink) {
    // A node of its own, separate from the one the camera controls use.
    let fd = match v4l2::open_node(device_path) {
        Ok(fd) => fd,
        Err(e) => {
            sink.set_error(friendly_error(e, device_path));
            return;
        }
    };

    let formats = list_formats(&fd);
    let Some(best) = preview_format::best_default(&formats).cloned() else {
        sink.set_error("The camera offers no video format this app can show.");
        return;
    };
    sink.set_formats(formats.clone(), best.key());
    let chosen = match preview_format::find(&formats, format_key) {
        Some(f) => f.clone(),
        None => {
            if !format_key.is_empty() {
                crate::log_warn!("Preview: the chosen format isn't offered by this camera; using the default.");
            }
            best
        }
    };
    let Some(pixelformat) = pixelformat_for(&chosen.fourcc) else {
        sink.set_error("The camera offers no video format this app can show.");
        return;
    };

    let negotiated = match set_format(&fd, &chosen, pixelformat) {
        Ok(n) => n,
        Err(e) => {
            sink.set_error(friendly_error(e, device_path));
            return;
        }
    };
    if fourcc_label(negotiated.pixelformat).is_none() || !safety::geometry(negotiated.width, negotiated.height) {
        sink.set_error("Preview failed: the camera chose a video format this app can't show.");
        return;
    }
    let rate = match set_frame_rate(&fd, &chosen) {
        Ok(rate) => rate,
        Err(e) => {
            crate::log_warn!("Preview: could not set the frame rate: {}", v4l2::errno_text(e));
            None
        }
    };

    let stream = match start_stream(fd) {
        Ok(s) => s,
        Err(e) => {
            sink.set_error(friendly_error(e, device_path));
            return;
        }
    };

    // Describe what the driver actually agreed to.
    let (fps_num, fps_den) = rate.unwrap_or((chosen.fps_num, chosen.fps_den));
    let actual = PreviewFormat {
        width: negotiated.width,
        height: negotiated.height,
        fps_num,
        fps_den,
        fourcc: fourcc_label(negotiated.pixelformat).unwrap_or_default().to_string(),
    };
    sink.set_current(actual.label(), chosen.key());
    crate::log_info!("Preview started ({}).", actual.label());

    let mut warned_bad_frame = false;
    while !sink.stop_requested() {
        match wait_for_frame(&stream.fd) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                sink.set_error(friendly_error(e, device_path));
                return;
            }
        }
        let mut buf = new_buffer();
        match v4l2::ioctl(&stream.fd, v4l2::VIDIOC_DQBUF, &mut buf) {
            Ok(()) => {}
            Err(libc::EAGAIN) => continue,
            Err(e) => {
                sink.set_error(friendly_error(e, device_path));
                return;
            }
        }
        if let Some(map) = stream.maps.get(buf.index as usize) {
            let damaged = buf.flags & v4l2::BUF_FLAG_ERROR != 0 || buf.bytesused == 0;
            let frame = if damaged { None } else { convert_frame(&negotiated, map.bytes(buf.bytesused as usize)) };
            match frame {
                Some((bgra, width, height)) => sink.publish(width, height, bgra),
                // A damaged or cut-short frame is skipped; the next one is usually fine.
                None if !warned_bad_frame => {
                    crate::log_warn!("Preview: skipped a damaged frame.");
                    warned_bad_frame = true;
                }
                None => {}
            }
        }
        if let Err(e) = v4l2::ioctl(&stream.fd, v4l2::VIDIOC_QBUF, &mut buf) {
            sink.set_error(friendly_error(e, device_path));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_from_intervals() {
        let f = |n, d| v4l2::Fract { numerator: n, denominator: d };
        assert_eq!(rate_from_interval(f(1, 30)), Some((30, 1)));
        // uvcvideo reports intervals in 100 ns units.
        assert_eq!(rate_from_interval(f(333_333, 10_000_000)), Some((10_000_000, 333_333)));
        assert_eq!(rate_from_interval(f(400_000, 10_000_000)), Some((25, 1)));
        assert_eq!(rate_from_interval(f(0, 30)), None);
        assert_eq!(rate_from_interval(f(1, 0)), None);
    }

    #[test]
    fn stepwise_ranges() {
        let s = v4l2::FrmSizeStepwise {
            min_width: 640,
            max_width: 1920,
            step_width: 16,
            min_height: 360,
            max_height: 1080,
            step_height: 8,
        };
        let sizes = stepwise_sizes(&s);
        assert_eq!(sizes[0], (1920, 1080));
        assert!(sizes.contains(&(1280, 720)));
        assert!(sizes.contains(&(640, 360)));
        assert!(!sizes.contains(&(3840, 2160)));
        // (1600 - 640) % 16 == 0 but (900 - 360) % 8 != 0.
        assert!(!sizes.contains(&(1600, 900)));

        let f = |n, d| v4l2::Fract { numerator: n, denominator: d };
        let range = v4l2::FrmIvalStepwise { min: f(1, 30), max: f(1, 5), step: f(1, 1) };
        assert_eq!(stepwise_rates(&range), vec![(30, 1), (25, 1), (24, 1), (15, 1), (10, 1), (5, 1)]);
        let broken = v4l2::FrmIvalStepwise { min: f(0, 0), max: f(0, 0), step: f(0, 0) };
        assert!(stepwise_rates(&broken).is_empty());
    }

    #[test]
    fn labels_round_trip() {
        for f in [v4l2::PIX_FMT_MJPEG, v4l2::PIX_FMT_YUYV, v4l2::PIX_FMT_NV12] {
            assert_eq!(pixelformat_for(fourcc_label(f).unwrap()), Some(f));
        }
        assert_eq!(fourcc_label(v4l2::fourcc(b"H264")), None);
        assert_eq!(pixelformat_for("H264"), None);
    }

    #[test]
    fn errors_are_friendly() {
        assert!(friendly_error(libc::EBUSY, "/dev/video0").starts_with("The camera is busy."));
        assert_eq!(friendly_error(libc::ENODEV, "/dev/video0"), "The camera was disconnected.");
        assert_eq!(friendly_error(libc::EIO, "/dev/video0"), "The camera was disconnected.");
        assert!(friendly_error(libc::EACCES, "/dev/video3").contains("/dev/video3"));
        assert_eq!(friendly_error(libc::EINVAL, "/dev/video0"), "Preview failed: EINVAL (Invalid argument)");
    }

    #[test]
    fn convert_uses_the_negotiated_stride() {
        // 2×1 YUYV in a 6-byte row: the driver's bytesperline, not width × 2.
        let n = Negotiated { width: 2, height: 1, pixelformat: v4l2::PIX_FMT_YUYV, bytesperline: 6 };
        let (px, w, h) = convert_frame(&n, &[235, 128, 16, 128, 0, 0]).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(px, [255, 255, 255, 255, 0, 0, 0, 255]);
        assert!(convert_frame(&n, &[235, 128, 16]).is_none());
    }
}
