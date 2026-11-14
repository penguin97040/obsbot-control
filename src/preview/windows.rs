//! Windows preview: a Media Foundation source reader converting the camera's native
//! format to RGB32. The reader runs asynchronously so a camera that stops answering
//! can never hold up shutdown: the loop waits for each frame in short steps and
//! checks the stop flag in between.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use windows::core::{implement, Interface, Ref, GUID, HRESULT, PWSTR};
use windows::Win32::Foundation::{E_ACCESSDENIED, E_FAIL};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer2, IMFActivate, IMFAttributes, IMFMediaEvent, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader,
    IMFSourceReaderCallback, IMFSourceReaderCallback_Impl, MF2DBuffer_LockFlags_Read, MFCreateAttributes,
    MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video, MFShutdown,
    MFStartup, MFVideoFormat_H264, MFVideoFormat_MJPG, MFVideoFormat_NV12, MFVideoFormat_RGB32, MFVideoFormat_YUY2,
    MFSTARTUP_LITE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_E_HW_MFT_FAILED_START_STREAMING,
    MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED, MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ASYNC_CALLBACK, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};

use super::Sink;
use crate::camera::windows::hr_text;
use crate::preview_format::{self, PreviewFormat};
use crate::safety;

/// How long one wait for a frame lasts before the stop flag is checked again.
const WAIT_STEP: Duration = Duration::from_millis(100);

/// A delivered sample, handed from Media Foundation's worker thread to ours.
struct Sample(IMFSample);

// SAFETY: Media Foundation samples are free-threaded COM objects (the reader runs in
// the multithreaded apartment), so moving one between threads is allowed.
unsafe impl Send for Sample {}

struct ReadResult {
    status: HRESULT,
    flags: u32,
    sample: Option<Sample>,
}

#[derive(Default)]
struct MailboxState {
    pending: bool,
    ready: bool,
    cancelled: bool,
    result: Option<ReadResult>,
}

/// One outstanding asynchronous read. The reader's callback owns a reference to this,
/// never to the capture loop, so late callbacks stay harmless after the loop ends.
#[derive(Default)]
struct Mailbox {
    state: Mutex<MailboxState>,
    ready: Condvar,
}

impl Mailbox {
    fn lock(&self) -> MutexGuard<'_, MailboxState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin_read(&self) -> bool {
        let mut s = self.lock();
        if s.cancelled || s.pending || s.ready {
            return false;
        }
        s.pending = true;
        true
    }

    fn deliver(&self, result: ReadResult) {
        let mut s = self.lock();
        if s.cancelled || !s.pending {
            return;
        }
        s.result = Some(result);
        s.pending = false;
        s.ready = true;
        self.ready.notify_all();
    }

    /// The delivered result, or None once cancelled or when `stop` turns true.
    fn wait(&self, stop: impl Fn() -> bool) -> Option<ReadResult> {
        let mut s = self.lock();
        loop {
            if s.cancelled || stop() {
                return None;
            }
            if s.ready {
                s.ready = false;
                return s.result.take();
            }
            s = self.ready.wait_timeout(s, WAIT_STEP).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn cancel(&self) {
        let discarded = {
            let mut s = self.lock();
            s.cancelled = true;
            self.ready.notify_all();
            s.result.take()
        };
        // Released outside the lock, allowing COM re-entrancy.
        drop(discarded);
    }
}

#[implement(IMFSourceReaderCallback)]
struct ReaderCallback {
    mailbox: Arc<Mailbox>,
}

impl IMFSourceReaderCallback_Impl for ReaderCallback_Impl {
    fn OnReadSample(
        &self,
        status: HRESULT,
        _stream: u32,
        flags: u32,
        _timestamp: i64,
        sample: Ref<'_, IMFSample>,
    ) -> windows::core::Result<()> {
        self.mailbox.deliver(ReadResult { status, flags, sample: sample.cloned().map(Sample) });
        Ok(())
    }

    fn OnFlush(&self, _stream: u32) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnEvent(&self, _stream: u32, event: Ref<'_, IMFMediaEvent>) -> windows::core::Result<()> {
        if let Some(event) = event.as_ref() {
            // SAFETY: a getter on the event the reader passed us.
            if let Ok(status) = unsafe { event.GetStatus() } {
                if status.is_err() {
                    self.mailbox.deliver(ReadResult { status, flags: 0, sample: None });
                }
            }
        }
        Ok(())
    }
}

/// DirectShow and Media Foundation expose the same camera under different
/// interface-class GUIDs, so compare the path up to the trailing "#{guid}".
fn instance_key(path: &str) -> String {
    let mut s = path.to_lowercase();
    if let Some(pos) = s.rfind("#{") {
        s.truncate(pos);
    }
    s
}

fn fourcc(subtype: &GUID) -> String {
    if *subtype == MFVideoFormat_MJPG {
        return "MJPG".into();
    }
    if *subtype == MFVideoFormat_NV12 {
        return "NV12".into();
    }
    if *subtype == MFVideoFormat_YUY2 {
        return "YUY2".into();
    }
    if *subtype == MFVideoFormat_H264 {
        return "H264".into();
    }
    if *subtype == MFVideoFormat_RGB32 {
        return "RGB32".into();
    }
    // A real FOURCC is four printable characters. Other subtypes (e.g. RGB24 is 20)
    // get their number, so two different ones can never share a format key.
    let bytes = subtype.data1.to_le_bytes();
    if bytes.iter().all(|b| (32..=126).contains(b)) {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        format!("0x{:08X}", subtype.data1)
    }
}

fn friendly_error(hr: HRESULT) -> String {
    if hr == E_ACCESSDENIED {
        return "Windows blocked camera access. Open Settings > Privacy & security > Camera and turn on \
                \"Let desktop apps access your camera\"."
            .into();
    }
    if hr == MF_E_HW_MFT_FAILED_START_STREAMING {
        return "The camera could not start. Another app (Teams, Zoom, OBS, the Camera app...) may be using it.".into();
    }
    if hr == MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED {
        return "The camera was disconnected or taken over by another app.".into();
    }
    if hr == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED {
        return "Another app took over the camera.".into();
    }
    format!("Preview failed: {}", hr_text(hr))
}

/// A UINT64 attribute packed as two UINT32s (high, low), like MFGetAttributeSize and
/// MFGetAttributeRatio; `default` is kept when the attribute is missing.
fn get_pair(attrs: &IMFAttributes, key: &GUID, default: (u32, u32)) -> (u32, u32) {
    // SAFETY: a getter on a live attribute store.
    match unsafe { attrs.GetUINT64(key) } {
        Ok(v) => ((v >> 32) as u32, v as u32),
        Err(_) => default,
    }
}

struct Native {
    format: PreviewFormat,
    key: String,
    media_type: IMFMediaType,
}

/// Frame geometry of the RGB32 output; zero width or height means "don't copy".
#[derive(Default)]
struct Geometry {
    width: u32,
    height: u32,
    default_stride: i32,
}

fn read_geometry(reader: &IMFSourceReader, stream: u32) -> Geometry {
    // SAFETY: a getter on a live reader.
    let Ok(current) = (unsafe { reader.GetCurrentMediaType(stream) }) else {
        return Geometry::default();
    };
    // SAFETY: a getter on a live media type.
    let Ok(size) = (unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }) else {
        return Geometry::default();
    };
    let (width, height) = ((size >> 32) as u32, size as u32);
    if !safety::geometry(width, height) {
        return Geometry::default();
    }
    // SAFETY: as above.
    match unsafe { current.GetGUID(&MF_MT_SUBTYPE) } {
        Ok(s) if s == MFVideoFormat_RGB32 => {}
        _ => return Geometry::default(),
    }
    // The stride attribute is a signed LONG stored as a UINT32.
    // SAFETY: as above.
    let default_stride = match unsafe { current.GetUINT32(&MF_MT_DEFAULT_STRIDE) } {
        Ok(s) => s as i32,
        Err(_) => (width * 4) as i32,
    };
    Geometry { width, height, default_stride }
}

/// Finds the Media Foundation source for this DirectShow device path. Reports its
/// own error and returns None on failure.
fn find_source(device_path: &str, sink: &Sink) -> Option<IMFMediaSource> {
    let key = instance_key(device_path);
    let mut source: Option<IMFMediaSource> = None;
    let mut reported = false;
    let mut attrs: Option<IMFAttributes> = None;
    // SAFETY: out pointer to a local Option.
    let mut hr = unsafe { MFCreateAttributes(&mut attrs, 1) };
    if let (Ok(()), Some(a)) = (&hr, &attrs) {
        // SAFETY: setting a GUID attribute on our own store.
        hr = unsafe { a.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID) };
    }
    if let (Ok(()), Some(a)) = (&hr, &attrs) {
        let mut devices: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: out pointers to locals; on success we own the array and each entry.
        if unsafe { MFEnumDeviceSources(a, &mut devices, &mut count) }.is_ok() && !devices.is_null() {
            for i in 0..count as usize {
                // SAFETY: i < count; taking the entry moves its reference out so it is
                // released when `device` drops.
                let device = unsafe { (*devices.add(i)).take() };
                let Some(device) = device else { continue };
                if source.is_some() {
                    continue;
                }
                let mut link = PWSTR::null();
                let mut len = 0u32;
                // SAFETY: out pointers to locals; the string is freed below.
                let got = unsafe {
                    device.GetAllocatedString(
                        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                        &mut link,
                        &mut len,
                    )
                };
                if got.is_ok() && !link.is_null() {
                    // SAFETY: GetAllocatedString returned a NUL-terminated string of `len` units.
                    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(link.0, len as usize) });
                    if instance_key(&text) == key {
                        // SAFETY: activating the matching device.
                        match unsafe { device.ActivateObject::<IMFMediaSource>() } {
                            Ok(s) => source = Some(s),
                            Err(e) => {
                                sink.set_error(friendly_error(e.code()));
                                reported = true;
                            }
                        }
                    }
                    // SAFETY: allocated by GetAllocatedString with CoTaskMemAlloc.
                    unsafe { CoTaskMemFree(Some(link.0 as *const _)) };
                }
            }
            // SAFETY: the array came from MFEnumDeviceSources and every entry is taken.
            unsafe { CoTaskMemFree(Some(devices as *const _)) };
        }
    }
    if source.is_none() && !reported {
        sink.set_error("Preview: the camera was not found by Media Foundation.");
    }
    source
}

fn create_reader(source: &IMFMediaSource, mailbox: &Arc<Mailbox>) -> windows::core::Result<IMFSourceReader> {
    let callback: IMFSourceReaderCallback = ReaderCallback { mailbox: mailbox.clone() }.into();
    let mut attrs: Option<IMFAttributes> = None;
    // SAFETY: out pointer to a local Option; the attributes are our own.
    unsafe {
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        attrs.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        MFCreateSourceReaderFromMediaSource(source, &attrs)
    }
}

/// Lists the native formats, picks the requested one (or the best for a preview),
/// sets it, then asks the reader to convert to RGB32. Returns the label of the format
/// now streaming, or None after reporting an error.
fn choose_format(reader: &IMFSourceReader, stream: u32, format_key: &str, sink: &Sink) -> Option<String> {
    let mut natives: Vec<Native> = Vec::new();
    for i in 0.. {
        // SAFETY: a getter on a live reader; fails past the last type.
        let Ok(t) = (unsafe { reader.GetNativeMediaType(stream, i) }) else {
            break;
        };
        let (width, height) = get_pair(&t, &MF_MT_FRAME_SIZE, (0, 0));
        let (fps_num, fps_den) = get_pair(&t, &MF_MT_FRAME_RATE, (0, 1));
        // SAFETY: as above.
        let sub = unsafe { t.GetGUID(&MF_MT_SUBTYPE) }.unwrap_or_default();
        // H.264 needs a decoder that may not be present; the camera offers the same
        // sizes uncompressed or as MJPG anyway.
        if sub == MFVideoFormat_H264 || !safety::geometry(width, height) {
            continue;
        }
        let format = PreviewFormat { width, height, fps_num, fps_den, fourcc: fourcc(&sub) };
        let key = format.key();
        if natives.iter().any(|n| n.key == key) {
            continue;
        }
        natives.push(Native { format, key, media_type: t });
    }

    // Unsorted: the picker helpers in preview_format order what they show.
    let list: Vec<PreviewFormat> = natives.iter().map(|n| n.format.clone()).collect();
    let best_key = preview_format::best_default(&list).map(|f| f.key()).unwrap_or_default();
    let best = natives.iter().find(|n| n.key == best_key);
    let mut chosen = best;
    if best.is_some() && !format_key.is_empty() {
        match natives.iter().find(|n| n.key == format_key) {
            Some(n) => chosen = Some(n),
            None => crate::log_warn!("Preview: the chosen format isn't offered by this camera; using the default."),
        }
    }
    sink.set_formats(list, best.map(|n| n.key.clone()).unwrap_or_default());

    if let Some(n) = chosen {
        // SAFETY: the type came from this reader's own native list.
        if let Err(e) = unsafe { reader.SetCurrentMediaType(stream, None, &n.media_type) } {
            sink.set_error(friendly_error(e.code()));
            return None;
        }
    }
    // SAFETY: building our own media type and handing it to the reader.
    let rgb = unsafe {
        MFCreateMediaType().and_then(|rgb| {
            rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(stream, None, &rgb)
        })
    };
    if let Err(e) = rgb {
        sink.set_error(friendly_error(e.code()));
        return None;
    }
    let (label, key) = match chosen {
        Some(n) => (n.format.label(), n.key.clone()),
        None => ("Camera default format".to_string(), String::new()),
    };
    sink.set_current(label.clone(), key);
    Some(label)
}

enum Copied {
    Frame(Vec<u8>),
    /// The buffer couldn't be read or doesn't really hold a whole frame.
    Skip,
    OutOfMemory,
}

/// Copies one RGB32 sample into tightly packed BGRA with opaque alpha.
fn copy_frame(sample: &IMFSample, g: &Geometry) -> Copied {
    match copy_frame_inner(sample, g) {
        Some(Some(bgra)) => Copied::Frame(bgra),
        Some(None) => Copied::OutOfMemory,
        None => Copied::Skip,
    }
}

/// None to skip the frame; Some(None) when the copy couldn't be allocated.
fn copy_frame_inner(sample: &IMFSample, g: &Geometry) -> Option<Option<Vec<u8>>> {
    let (width, height) = (g.width, g.height);
    let row_bytes = width as usize * 4;
    // SAFETY: getters on the sample the reader delivered.
    let buffer = unsafe {
        match sample.GetBufferCount().ok()? {
            1 => sample.GetBufferByIndex(0),
            _ => sample.ConvertToContiguousBuffer(),
        }
    }
    .ok()?;

    let mut raw: *mut u8 = std::ptr::null_mut();
    let mut scan0: *mut u8 = std::ptr::null_mut();
    let mut allocation = 0u32;
    let mut pitch = 0i32;
    let scan_offset: u64;
    let buffer2d = buffer.cast::<IMF2DBuffer2>().ok();
    let locked2d = buffer2d.as_ref().is_some_and(|b| {
        // SAFETY: out pointers to locals; a successful lock is undone by `unlock` below.
        unsafe { b.Lock2DSize(MF2DBuffer_LockFlags_Read, &mut scan0, &mut pitch, &mut raw, &mut allocation) }.is_ok()
    });
    let unlock = || {
        // SAFETY: only called after the matching successful lock.
        unsafe {
            match (&buffer2d, locked2d) {
                (Some(b), true) => {
                    let _ = b.Unlock2D();
                }
                _ => {
                    let _ = buffer.Unlock();
                }
            }
        }
    };
    if locked2d {
        let (start, scan) = (raw as usize, scan0 as usize);
        if raw.is_null() || scan0.is_null() || scan < start {
            unlock();
            return None;
        }
        scan_offset = (scan - start) as u64;
    } else {
        let mut max_len = 0u32;
        // SAFETY: out pointers to locals; unlocked by `unlock` below.
        if unsafe { buffer.Lock(&mut raw, Some(&mut max_len), Some(&mut allocation)) }.is_err() {
            return None;
        }
        pitch = g.default_stride;
        let stride = pitch.unsigned_abs() as u64;
        scan_offset = if pitch < 0 { stride * (height as u64 - 1) } else { 0 };
        if raw.is_null() || allocation > max_len {
            unlock();
            return None;
        }
    }
    if !safety::frame_bounds(width, height, pitch, scan_offset, allocation as u64) {
        unlock();
        return None;
    }

    let mut out = Vec::new();
    if out.try_reserve_exact(row_bytes * height as usize).is_err() {
        unlock();
        return Some(None);
    }
    out.resize(row_bytes * height as usize, 0);
    for y in 0..height as usize {
        // SAFETY: frame_bounds checked that every row (scan_offset + pitch * y, for
        // row_bytes bytes) lies inside the locked allocation starting at raw.
        let src = unsafe {
            let row = raw.add(scan_offset as usize).offset(pitch as isize * y as isize);
            std::slice::from_raw_parts(row as *const u8, row_bytes)
        };
        let dst = &mut out[y * row_bytes..(y + 1) * row_bytes];
        dst.copy_from_slice(src);
        // RGB32 alpha is undefined.
        for px in dst.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
    }
    unlock();
    Some(Some(out))
}

/// Streams frames until asked to stop or something fails. Returns once the loop ends;
/// the caller cleans up.
fn capture(reader: &IMFSourceReader, stream: u32, mailbox: &Mailbox, sink: &Sink) {
    // Re-read whenever the reader reports a format change, so a new frame size can
    // never overrun the copy.
    let mut geometry = read_geometry(reader, stream);
    while !sink.stop_requested() {
        if !mailbox.begin_read() {
            break;
        }
        // All output pointers must be null for an asynchronous source reader.
        // SAFETY: an asynchronous read; the result arrives through the callback.
        if let Err(e) = unsafe { reader.ReadSample(stream, 0, None, None, None, None) } {
            mailbox.deliver(ReadResult { status: e.code(), flags: 0, sample: None });
        }
        let Some(result) = mailbox.wait(|| sink.stop_requested()) else {
            break;
        };
        if result.status.is_err() || result.flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            sink.set_error(friendly_error(if result.status.is_err() { result.status } else { E_FAIL }));
            break;
        }
        if result.flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            sink.set_error("The camera stopped sending video.");
            break;
        }
        if result.flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            geometry = read_geometry(reader, stream);
        }
        let Some(Sample(sample)) = result.sample else { continue };
        if geometry.width == 0 || geometry.height == 0 {
            continue;
        }
        match copy_frame(&sample, &geometry) {
            Copied::Frame(bgra) => sink.publish(geometry.width, geometry.height, bgra),
            Copied::Skip => {}
            Copied::OutOfMemory => {
                sink.set_error("Preview failed: there was not enough memory to capture a frame.");
                break;
            }
        }
    }
}

pub fn run(device_path: &str, format_key: &str, sink: &Sink) {
    // SAFETY: this thread joins the multithreaded apartment; balanced below.
    let hr_com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr_com.is_err() {
        sink.set_error(friendly_error(hr_com));
        return;
    }
    // Media Foundation keeps a start-up count, so this pairs safely with any other user.
    // SAFETY: balanced by MFShutdown below.
    let mf = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) };
    if let Err(e) = &mf {
        sink.set_error(friendly_error(e.code()));
    }

    let mailbox = Arc::new(Mailbox::default());
    let source = if mf.is_ok() { find_source(device_path, sink) } else { None };
    let mut reader = None;
    if let Some(source) = &source {
        match create_reader(source, &mailbox) {
            Ok(r) => reader = Some(r),
            Err(e) => sink.set_error(friendly_error(e.code())),
        }
    }
    if let Some(r) = &reader {
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        if let Some(label) = choose_format(r, stream, format_key, sink) {
            crate::log_info!("Preview started ({label}).");
            capture(r, stream, &mailbox, sink);
        }
    }

    mailbox.cancel();
    if let Some(r) = reader.take() {
        // SAFETY: flushing our own reader; any late callback lands in the cancelled mailbox.
        let _ = unsafe { r.Flush(MF_SOURCE_READER_ALL_STREAMS.0 as u32) };
    }
    if let Some(s) = source {
        // SAFETY: shutting down the source we activated.
        let _ = unsafe { s.Shutdown() };
    }
    if mf.is_ok() {
        // SAFETY: balances the successful MFStartup above.
        let _ = unsafe { MFShutdown() };
    }
    // SAFETY: balances the successful CoInitializeEx above, on the same thread.
    unsafe { CoUninitialize() };
}

#[cfg(test)]
mod tests {
    use super::instance_key;

    #[test]
    fn instance_key_ignores_interface_class_and_case() {
        let ds = r"\\?\USB#VID_3564&PID_FEF9&MI_00#6&1a2b&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        let mf = r"\\?\usb#vid_3564&pid_fef9&mi_00#6&1a2b&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global";
        assert_eq!(instance_key(ds), instance_key(mf));
        assert_eq!(instance_key("ABC"), "abc");
    }
}
