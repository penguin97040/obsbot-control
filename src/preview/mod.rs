//! The live preview: a background thread reads frames from the camera with the
//! platform's capture API and hands the newest one to the interface as BGRA pixels.

#[cfg(target_os = "linux")]
pub mod convert;
pub mod demo;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use crate::preview_format::PreviewFormat;
use crate::still_image;

/// One video frame: tightly packed BGRA (blue, green, red, alpha) bytes.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    frame: Option<Frame>,
    frame_id: u64,
    error: String,
    format_label: String,
    format_key: String,
    formats: Vec<PreviewFormat>,
    default_key: String,
    format_generation: u64,
    /// A small blurred copy of the latest frame, kept for "Preview paused".
    still: Option<Frame>,
    still_shape: (u32, u32),
    still_from: u64,
}

/// What a platform's capture loop reports through. Cheap to clone.
#[derive(Clone)]
pub struct Sink {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Sink {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The capture loop must return promptly (within about 100 ms) once this is true.
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// The camera's native formats, in the order offered (H.264 left out), and the
    /// key of the one that suits a preview best.
    pub fn set_formats(&self, formats: Vec<PreviewFormat>, default_key: String) {
        let mut s = self.lock();
        s.formats = formats;
        s.default_key = default_key;
        s.format_generation += 1;
    }

    /// The format now streaming: a label such as "1280×720 MJPG 30 fps" and its key.
    pub fn set_current(&self, label: String, key: String) {
        let mut s = self.lock();
        s.format_label = label;
        s.format_key = key;
        s.format_generation += 1;
    }

    /// Records a readable reason the preview stopped (and logs it). The capture
    /// loop should return after this.
    pub fn set_error(&self, message: impl Into<String>) {
        let message = message.into();
        crate::log_warn!("{message}");
        self.lock().error = message;
    }

    /// Publishes a frame of tightly packed BGRA pixels (width × height × 4 bytes).
    /// Frames of the wrong size are dropped.
    pub fn publish(&self, width: u32, height: u32, bgra: Vec<u8>) {
        if !crate::safety::geometry(width, height) || bgra.len() != width as usize * height as usize * 4 {
            return;
        }
        if self.stop_requested() {
            return;
        }
        let mut s = self.lock();
        s.frame = Some(Frame { width, height, bgra });
        s.frame_id += 1;
    }
}

/// Runs the platform capture loop for one camera until it fails or is asked to stop.
/// `format_key` is a PreviewFormat::key(); empty or unknown picks the default.
fn run_capture(device_path: &str, format_key: &str, sink: &Sink) {
    if device_path == demo::DEMO_PATH {
        demo::run(format_key, sink);
        return;
    }
    #[cfg(target_os = "linux")]
    linux::run(device_path, format_key, sink);
    #[cfg(windows)]
    windows::run(device_path, format_key, sink);
    #[cfg(not(any(target_os = "linux", windows)))]
    sink.set_error("Preview isn't available on this system.");
}

/// Owns the capture thread. Used from the interface thread only.
pub struct Preview {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: String,
    requested_format: String,
}

impl Default for Preview {
    fn default() -> Self {
        Preview {
            shared: Arc::new(Mutex::new(Shared::default())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            path: String::new(),
            requested_format: String::new(),
        }
    }
}

impl Preview {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn start(&mut self, device_path: &str, format_key: &str) {
        self.stop();
        {
            let mut s = self.lock();
            if device_path != self.path {
                s.formats.clear();
                s.default_key.clear();
                s.still = None;
                s.still_from = 0;
            }
            s.error.clear();
            s.format_label.clear();
            s.format_key.clear();
            s.format_generation += 1;
            s.frame = None;
        }
        self.path = device_path.to_string();
        self.requested_format = format_key.to_string();
        self.stop = Arc::new(AtomicBool::new(false));
        let sink = Sink { shared: self.shared.clone(), stop: self.stop.clone() };
        let path = device_path.to_string();
        let key = format_key.to_string();
        let spawned = std::thread::Builder::new().name("preview".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_capture(&path, &key, &sink)));
            if result.is_err() {
                sink.set_error("Preview failed unexpectedly.");
            }
            crate::log_info!("Preview stopped.");
        });
        match spawned {
            Ok(handle) => self.thread = Some(handle),
            Err(_) => self.lock().error = "Preview failed: could not start a thread.".into(),
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    /// True while the capture thread is running.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    pub fn device_path(&self) -> &str {
        &self.path
    }

    pub fn requested_format(&self) -> &str {
        &self.requested_format
    }

    /// The newest frame since the last call, if any.
    pub fn take_frame(&mut self) -> Option<Frame> {
        self.lock().frame.take()
    }

    /// Non-empty when the preview stopped because of a problem.
    pub fn error(&self) -> String {
        self.lock().error.clone()
    }

    /// e.g. "1280×720 MJPG 30 fps"
    pub fn format_label(&self) -> String {
        self.lock().format_label.clone()
    }

    pub fn current_format_key(&self) -> String {
        self.lock().format_key.clone()
    }

    /// The camera's native formats, kept from the last time it was opened.
    pub fn formats(&self) -> Vec<PreviewFormat> {
        self.lock().formats.clone()
    }

    pub fn default_format_key(&self) -> String {
        self.lock().default_key.clone()
    }

    /// Changes whenever formats(), default_format_key() or current_format_key() may have.
    pub fn format_generation(&self) -> u64 {
        self.lock().format_generation
    }

    /// Keeps a blurred copy of this frame (BGRA), shown while the preview is paused.
    pub fn remember_still(&mut self, frame_id: u64, width: u32, height: u32, bgra: &[u8]) {
        let mut s = self.lock();
        if s.still_from == frame_id {
            return;
        }
        if let Some((pixels, w, h)) = still_image::blurred_thumbnail(bgra, width, height, 48) {
            s.still = Some(Frame { width: w, height: h, bgra: pixels });
            s.still_shape = (width, height);
            s.still_from = frame_id;
        }
    }

    /// The blurred still and the shape of the frame it came from.
    pub fn still(&self) -> Option<(Vec<u8>, u32, u32, (u32, u32))> {
        let s = self.lock();
        s.still.as_ref().map(|f| (f.bgra.clone(), f.width, f.height, s.still_shape))
    }

    pub fn clear_still(&mut self) {
        let mut s = self.lock();
        s.still = None;
        s.still_from = 0;
    }

    /// Asks the thread to stop without waiting for it (used when the app exits).
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
    }
}
