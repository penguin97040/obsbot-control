//! CameraWorker: owns the camera on a background thread. The interface posts
//! commands (never blocking) and reads a copied CameraState snapshot.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{CameraBackend, DeviceInfo, UvcCtl, UvcInfo, POWER_LINE_50HZ, POWER_LINE_60HZ};
use crate::protocol::{self, AiMode, Fov, FrameReply, Model, Receiver, Status, Variant, XuBuffer, XU_LENGTH};
use crate::safety::{self, GimbalMotion};
use crate::{log_error, log_info, log_warn};

const SCAN_INTERVAL: Duration = Duration::from_millis(2000);
const STATUS_INTERVAL: Duration = Duration::from_millis(1000);
const PRESENCE_INTERVAL: Duration = Duration::from_millis(3000);
const GIMBAL_RESEND: Duration = Duration::from_millis(100);
/// Hold-to-move stops by itself if the interface stops refreshing it for this long.
const GIMBAL_DEAD_MAN: Duration = Duration::from_millis(250);
const REPLY_POLLS: u32 = 8;
const REPLY_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Which protocol codes to use where the sources disagree (see docs/PROTOCOL.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantSetting {
    Auto = 0,
    Tiny2 = 1,
    Lite = 2,
}

impl VariantSetting {
    pub fn from_index(i: i32) -> VariantSetting {
        match i {
            1 => VariantSetting::Tiny2,
            2 => VariantSetting::Lite,
            _ => VariantSetting::Auto,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CameraState {
    pub connected: bool,
    pub device: DeviceInfo,
    pub available: Vec<DeviceInfo>,
    pub xu: bool,
    pub xu_location: u32,
    pub variant: Variant,
    pub uvc: [UvcInfo; UvcCtl::COUNT],
    pub status: Status,
    pub raw_status: XuBuffer,
    /// The last Developer-tab result.
    pub dev_output: String,
}

impl Default for CameraState {
    fn default() -> Self {
        CameraState {
            connected: false,
            device: DeviceInfo::default(),
            available: Vec::new(),
            xu: false,
            xu_location: 0,
            variant: Variant::Tiny2,
            uvc: [UvcInfo::default(); UvcCtl::COUNT],
            status: Status::default(),
            raw_status: [0; XU_LENGTH],
            dev_output: String::new(),
        }
    }
}

impl CameraState {
    pub fn uvc(&self, c: UvcCtl) -> &UvcInfo {
        &self.uvc[c.index()]
    }
}

type Task = Box<dyn FnOnce(&mut Inner) + Send>;

#[derive(Default)]
struct GimbalWant {
    pitch: f32,
    yaw: f32,
    refreshed: Option<Instant>,
}

struct Shared {
    queue: Mutex<VecDeque<(String, Task)>>,
    queue_cv: Condvar,
    state: Mutex<CameraState>,
    generation: AtomicU64,
    running: AtomicBool,
    switching: AtomicBool,
    variant_setting: AtomicI32,
    /// The anti-flicker value to apply on connecting; -1 leaves the camera alone.
    anti_flicker: AtomicI32,
    /// The preferred camera path and a counter bumped on every selection.
    preference: Mutex<(String, u64)>,
    gimbal: Mutex<GimbalWant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn update_state(&self, f: impl FnOnce(&mut CameraState)) {
        f(&mut lock(&self.state));
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> CameraState {
        lock(&self.state).clone()
    }

    fn clear_gimbal_hold(&self) {
        *lock(&self.gimbal) = GimbalWant::default();
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn switching(&self) -> bool {
        self.switching.load(Ordering::Relaxed)
    }
}

pub type BackendFactory = Box<dyn FnOnce() -> Box<dyn CameraBackend> + Send>;

pub struct CameraWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl CameraWorker {
    pub fn new() -> Self {
        CameraWorker {
            shared: Arc::new(Shared {
                queue: Mutex::new(VecDeque::new()),
                queue_cv: Condvar::new(),
                state: Mutex::new(CameraState::default()),
                generation: AtomicU64::new(1),
                running: AtomicBool::new(false),
                switching: AtomicBool::new(false),
                variant_setting: AtomicI32::new(0),
                anti_flicker: AtomicI32::new(-1),
                preference: Mutex::new((String::new(), 0)),
                gimbal: Mutex::new(GimbalWant::default()),
            }),
            thread: None,
        }
    }

    /// Starts the thread. The backend is made on that thread, so it may hold
    /// thread-bound handles (COM objects on Windows).
    pub fn start(&mut self, make_backend: BackendFactory) {
        if self.shared.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new().name("camera".into()).spawn(move || {
            let mut inner = Inner {
                shared,
                backend: make_backend(),
                gimbal: GimbalMotion::default(),
                last_gimbal_send: Instant::now(),
                seq: 1,
            };
            inner.run();
        });
        match spawned {
            Ok(handle) => self.thread = Some(handle),
            Err(_) => {
                self.shared.running.store(false, Ordering::SeqCst);
                log_error!("Could not start the camera thread.");
            }
        }
    }

    pub fn stop(&mut self) {
        if !self.shared.running.swap(false, Ordering::SeqCst) {
            return;
        }
        self.shared.queue_cv.notify_all();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    pub fn snapshot(&self) -> CameraState {
        self.shared.snapshot()
    }

    /// Changes whenever the snapshot may have.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }

    fn post(&self, key: &str, task: impl FnOnce(&mut Inner) + Send + 'static) {
        {
            let mut queue = lock(&self.shared.queue);
            if !key.is_empty() {
                if let Some(entry) = queue.iter_mut().find(|(k, _)| k == key) {
                    // A newer value replaces a queued one (e.g. while dragging a slider).
                    entry.1 = Box::new(task);
                    return;
                }
            }
            queue.push_back((key.to_string(), Box::new(task)));
        }
        self.shared.queue_cv.notify_one();
    }

    // ---- Settings that affect behaviour; safe to call any time ---------------------

    pub fn set_variant_setting(&self, v: VariantSetting) {
        self.shared.variant_setting.store(v as i32, Ordering::Relaxed);
        self.post("variant", |w| {
            let variant = w.effective_variant();
            w.shared.update_state(|s| s.variant = variant);
        });
    }

    pub fn set_anti_flicker_on_connect(&self, power_line_value: i32) {
        self.shared.anti_flicker.store(power_line_value, Ordering::Relaxed);
    }

    pub fn select_device(&self, path: &str) {
        {
            let mut pref = lock(&self.shared.preference);
            pref.0 = path.to_string();
            pref.1 += 1;
            self.shared.switching.store(true, Ordering::SeqCst);
        }
        self.shared.clear_gimbal_hold();
        self.post("selection", |_| {});
    }

    // ---- Asynchronous commands -------------------------------------------------------

    pub fn set_uvc(&self, c: UvcCtl, value: i32, is_auto: bool) {
        let info = *self.shared.snapshot().uvc(c);
        if !info.supported {
            return;
        }
        let value = value.clamp(info.min, info.max);
        // Update the snapshot straight away so sliders don't jump back while queued.
        self.shared.update_state(|s| {
            let u = &mut s.uvc[c.index()];
            u.value = value;
            u.is_auto = is_auto;
        });
        self.post(&format!("uvc:{}", c.index()), move |w| {
            if !w.backend.is_open() {
                return;
            }
            let result = w.backend.set_uvc(c, value, is_auto);
            if let Err(e) = &result {
                log_error!("Setting {} failed: {e}", c.log_name());
            }
            if is_auto || result.is_err() {
                if let Some((v, a)) = w.backend.get_uvc(c) {
                    w.shared.update_state(|s| {
                        s.uvc[c.index()].value = v;
                        s.uvc[c.index()].is_auto = a;
                    });
                }
            }
        });
    }

    pub fn reset_image_defaults(&self) {
        self.post("", |w| {
            if !w.backend.is_open() {
                return;
            }
            for c in [
                UvcCtl::Brightness,
                UvcCtl::Contrast,
                UvcCtl::Hue,
                UvcCtl::Saturation,
                UvcCtl::Sharpness,
                UvcCtl::Gamma,
                UvcCtl::Gain,
                UvcCtl::BacklightComp,
                UvcCtl::WhiteBalance,
                UvcCtl::Exposure,
                UvcCtl::Focus,
            ] {
                if let Some(info) = w.backend.query_uvc(c) {
                    let _ = w.backend.set_uvc(c, info.def, info.can_auto);
                }
            }
            w.query_all_uvc();
            log_info!("Image settings reset to camera defaults.");
        });
    }

    pub fn set_ai_mode(&self, m: AiMode) {
        self.shared.update_state(|s| s.status.ai_mode = Some(m));
        self.post("ai", move |w| {
            w.send_simple(&format!("AI tracking \u{2192} {}", m.name()), &m.command());
        });
    }

    pub fn set_sport_tracking(&self, sport: bool) {
        self.shared.update_state(|s| s.status.sport_tracking = sport);
        self.post("trackspeed", move |w| {
            let what = if sport { "Tracking speed \u{2192} sport" } else { "Tracking speed \u{2192} standard" };
            w.send_framed(what, Receiver::Ai, protocol::cmd::TRACKING_SPEED, &[if sport { 2 } else { 0 }], true);
        });
    }

    pub fn set_hdr(&self, on: bool) {
        self.shared.update_state(|s| s.status.hdr = on);
        self.post("hdr", move |w| {
            w.send_simple(if on { "HDR on" } else { "HDR off" }, &protocol::hdr_command(on));
        });
    }

    pub fn set_fov(&self, f: Fov) {
        self.post("fov", move |w| {
            let v = w.effective_variant();
            if !w.send_simple(&format!("Field of view \u{2192} {}", f.name()), &f.command(v)) {
                return;
            }
            w.pause(Duration::from_millis(300));
            w.read_status();
            let st = w.shared.snapshot().status;
            if st.valid && Fov::from_status(st.fov_raw, v) != Some(f) {
                log_warn!(
                    "Field of view status reads {}. If the picture didn't change, try the other protocol variant on the Developer tab.",
                    st.fov_raw
                );
            }
        });
    }

    pub fn set_sleep(&self, sleep: bool) {
        self.post("sleep", move |w| {
            let what = if sleep { "Sleep" } else { "Wake" };
            if w.effective_variant() == Variant::Lite {
                w.send_simple(what, &protocol::simple_sleep_command(sleep));
            } else {
                let payload = [sleep as u8, 0, 0, 0];
                let ok = w.send_framed(what, Receiver::Camera, protocol::cmd::SLEEP_WAKE, &payload, true).is_some();
                if !ok && w.shared.running() && !w.shared.switching() && !w.gimbal.moving {
                    log_info!("Trying the alternative {} command...", if sleep { "sleep" } else { "wake" });
                    w.send_simple(what, &protocol::simple_sleep_command(sleep));
                }
            }
            w.pause(Duration::from_millis(500));
            w.read_status();
        });
    }

    pub fn recentre(&self) {
        self.post("recentre", |w| {
            if w.send_framed("Centre gimbal", Receiver::Gimbal, protocol::cmd::RECENTRE, &[0; 6], true).is_none() {
                if !w.shared.running() || w.shared.switching() || w.gimbal.moving {
                    return;
                }
                log_info!("Trying the alternative centre command...");
                w.send_simple("Centre gimbal", &protocol::simple_recentre_command());
            }
        });
    }

    pub fn goto_position(&self, pan: i32, tilt: i32, zoom: i32) {
        self.post("goto", move |w| {
            if !w.backend.is_open() {
                return;
            }
            let a = w.backend.set_uvc(UvcCtl::Pan, pan, false);
            let b = w.backend.set_uvc(UvcCtl::Tilt, tilt, false);
            let c = w.backend.set_uvc(UvcCtl::Zoom, zoom, false);
            if a.is_err() || b.is_err() || c.is_err() {
                let word = |r: &Result<(), String>| if r.is_ok() { "ok" } else { "failed" };
                log_warn!("Preset recall partly failed (pan {}, tilt {}, zoom {}).", word(&a), word(&b), word(&c));
            } else {
                log_info!("Moved to preset (pan {pan}, tilt {tilt}, zoom {zoom}).");
            }
            // Read back the camera's values, including clamping and partial failures.
            w.query_all_uvc();
        });
    }

    pub fn refresh_all(&self) {
        self.post("refresh", |w| {
            if !w.backend.is_open() {
                return;
            }
            w.query_all_uvc();
            w.read_status();
            log_info!("Refreshed camera values.");
        });
    }

    /// Hold-to-move. Call about every frame while a direction is held; movement stops
    /// by itself if the calls stop for longer than the dead-man timeout.
    pub fn hold_gimbal_velocity(&self, pitch_deg_per_sec: f32, yaw_deg_per_sec: f32) {
        if self.shared.switching() {
            return;
        }
        if !pitch_deg_per_sec.is_finite() || !yaw_deg_per_sec.is_finite() {
            self.shared.clear_gimbal_hold();
            self.shared.queue_cv.notify_one();
            return;
        }
        {
            let mut g = lock(&self.shared.gimbal);
            if self.shared.switching() {
                return;
            }
            g.pitch = safety::velocity(pitch_deg_per_sec);
            g.yaw = safety::velocity(yaw_deg_per_sec);
            g.refreshed = Some(Instant::now());
        }
        self.shared.queue_cv.notify_one();
    }

    /// Stops hold-to-move straight away (the button was released).
    pub fn release_gimbal(&self) {
        self.shared.clear_gimbal_hold();
        self.shared.queue_cv.notify_one();
    }

    // ---- Developer tab -----------------------------------------------------------------

    pub fn dev_set_raw(&self, selector: u8, bytes: Vec<u8>) {
        if bytes.len() > XU_LENGTH {
            log_warn!("Developer SET rejected: at most 60 bytes are allowed.");
            self.shared.update_state(|s| s.dev_output = "SET rejected: at most 60 bytes are allowed.".into());
            return;
        }
        self.post("", move |w| {
            let mut buf = [0u8; XU_LENGTH];
            buf[..bytes.len()].copy_from_slice(&bytes);
            let result = w.backend.xu_set(selector, &buf);
            let out = format!(
                "SET selector {selector}: {}\n{}",
                result.err().unwrap_or_else(|| "OK".into()),
                protocol::to_hex(&buf)
            );
            log_info!("[dev] {out}");
            w.shared.update_state(|s| s.dev_output = out);
        });
    }

    pub fn dev_get_raw(&self, selector: u8) {
        self.post("", move |w| {
            let (text, buf) = match w.backend.xu_get(selector) {
                Ok(buf) => ("OK".to_string(), buf),
                Err(e) => (e, [0u8; XU_LENGTH]),
            };
            let out = format!("GET selector {selector}: {text}\n{}", protocol::to_hex(&buf));
            log_info!("[dev] {out}");
            w.shared.update_state(|s| s.dev_output = out);
        });
    }

    pub fn dev_framed(&self, receiver: Receiver, command: u16, payload: Vec<u8>) {
        if payload.len() > protocol::MAX_PAYLOAD {
            log_warn!("Developer frame rejected: at most 44 payload bytes are allowed.");
            self.shared.update_state(|s| s.dev_output = "Frame rejected: at most 44 payload bytes are allowed.".into());
            return;
        }
        self.post("", move |w| {
            let what = format!("[dev] framed command 0x{command:04X}");
            let out = match w.send_framed(&what, receiver, command, &payload, true) {
                Some(reply) => format!(
                    "{what}: reply flags 0x{:02X}\npayload: {}",
                    reply.flags,
                    protocol::to_hex(&reply.payload)
                ),
                None => format!("{what}: no reply"),
            };
            w.shared.update_state(|s| s.dev_output = out);
        });
    }
}

impl Drop for CameraWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The worker thread's side: owns the backend.
struct Inner {
    shared: Arc<Shared>,
    backend: Box<dyn CameraBackend>,
    gimbal: GimbalMotion,
    last_gimbal_send: Instant,
    seq: u16,
}

impl Inner {
    fn run(&mut self) {
        if let Err(e) = self.backend.thread_start() {
            log_error!("Camera worker startup failed: {e}");
            return;
        }
        log_info!("Camera worker started.");
        let mut last_scan: Option<Instant> = None;
        let mut last_status = Instant::now();
        let mut last_presence = Instant::now();

        while self.shared.running() {
            let tasks: VecDeque<(String, Task)> = {
                let queue = lock(&self.shared.queue);
                // Wake often enough to service hold-to-move smoothly.
                let wait = Duration::from_millis(if self.gimbal.moving { 20 } else { 100 });
                let (mut queue, _) = self
                    .shared
                    .queue_cv
                    .wait_timeout_while(queue, wait, |q| q.is_empty() && self.shared.running())
                    .unwrap_or_else(|e| e.into_inner());
                std::mem::take(&mut *queue)
            };
            if !self.shared.running() {
                break;
            }
            let mut tasks = tasks;
            if self.shared.switching() {
                let generation = lock(&self.shared.preference).1;
                if !self.backend.is_open() || self.disconnect("switching camera", true) {
                    self.try_connect();
                    // A newer selection made while connecting must still be serviced.
                    let pref = lock(&self.shared.preference);
                    if pref.1 == generation {
                        self.shared.switching.store(false, Ordering::SeqCst);
                    }
                }
                // Queued commands belong to the previous camera.
                tasks.clear();
            }
            for (_, task) in tasks {
                if !self.shared.running() {
                    break;
                }
                task(self);
                // Keep the hold-to-move dead-man responsive between queued tasks.
                if self.backend.is_open() {
                    self.service_gimbal();
                }
            }
            if !self.shared.running() {
                break;
            }

            let now = Instant::now();
            if !self.backend.is_open() {
                if last_scan.is_none_or(|t| now - t >= SCAN_INTERVAL) {
                    last_scan = Some(now);
                    self.try_connect();
                    last_status = Instant::now();
                    last_presence = last_status;
                }
                continue;
            }

            self.service_gimbal();

            if now - last_status >= STATUS_INTERVAL {
                last_status = now;
                self.read_status();
                for c in [UvcCtl::Pan, UvcCtl::Tilt, UvcCtl::Zoom] {
                    if let Some((v, _)) = self.backend.get_uvc(c) {
                        let changed = self.shared.snapshot().uvc(c).value != v;
                        if changed {
                            self.shared.update_state(|s| s.uvc[c.index()].value = v);
                        }
                    }
                }
            }
            if now - last_presence >= PRESENCE_INTERVAL {
                last_presence = now;
                let path = self.shared.snapshot().device.path;
                let present = self.backend.enumerate().iter().any(|d| d.path.eq_ignore_ascii_case(&path));
                if !present {
                    self.disconnect("camera unplugged", false);
                }
            }
        }

        self.shared.clear_gimbal_hold();
        if self.backend.is_open() {
            for _ in 0..3 {
                if self.stop_gimbal() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if self.gimbal.moving {
                log_warn!("Could not stop the gimbal before closing the camera.");
            }
            self.backend.close();
        }
        log_info!("Camera worker stopped.");
        self.backend.thread_end();
    }

    fn effective_variant(&self) -> Variant {
        match VariantSetting::from_index(self.shared.variant_setting.load(Ordering::Relaxed)) {
            VariantSetting::Tiny2 => Variant::Tiny2,
            VariantSetting::Lite => Variant::Lite,
            VariantSetting::Auto => {
                if self.shared.snapshot().device.model == Model::Tiny2Lite {
                    Variant::Lite
                } else {
                    Variant::Tiny2
                }
            }
        }
    }

    fn try_connect(&mut self) {
        let found = self.backend.enumerate();
        let changed = self.shared.snapshot().available != found;
        if changed {
            let list = found.clone();
            self.shared.update_state(|s| s.available = list);
        }
        if found.is_empty() {
            return;
        }
        let preferred = lock(&self.shared.preference).0.clone();
        let pick = found
            .iter()
            .find(|d| !preferred.is_empty() && d.path.eq_ignore_ascii_case(&preferred))
            .unwrap_or(&found[0])
            .clone();

        log_info!("Connecting to {} (VID {:04X}, PID {:04X})...", pick.name, pick.vid, pick.pid);
        if pick.model == Model::Unknown {
            log_warn!("This OBSBOT model is untested. Controls may not work.");
        }
        if !self.backend.open(&pick) {
            return;
        }
        self.shared.clear_gimbal_hold();
        self.gimbal.moving = false;

        let xu = self.backend.has_xu();
        let xu_location = self.backend.xu_location();
        let device = pick.clone();
        self.shared.update_state(|s| {
            s.connected = true;
            s.device = device;
            s.xu = xu;
            s.xu_location = xu_location;
        });
        let variant = self.effective_variant();
        self.shared.update_state(|s| s.variant = variant);
        self.query_all_uvc();

        let af = self.shared.anti_flicker.load(Ordering::Relaxed);
        if (0..=POWER_LINE_60HZ).contains(&af) {
            let info = *self.shared.snapshot().uvc(UvcCtl::PowerLine);
            if info.supported && info.value != af {
                match self.backend.set_uvc(UvcCtl::PowerLine, af, false) {
                    Ok(()) => {
                        let name = match af {
                            POWER_LINE_50HZ => "50 Hz",
                            POWER_LINE_60HZ => "60 Hz",
                            _ => "off",
                        };
                        log_info!("Applied anti-flicker setting ({name}).");
                        self.shared.update_state(|s| s.uvc[UvcCtl::PowerLine.index()].value = af);
                    }
                    Err(e) => log_warn!("Could not apply anti-flicker setting: {e}"),
                }
            }
        }
        self.read_status();
        log_info!("Connected.");
    }

    /// Closes the camera. With `require_stop`, only if the gimbal is known to have stopped.
    fn disconnect(&mut self, reason: &str, require_stop: bool) -> bool {
        self.shared.clear_gimbal_hold();
        if require_stop {
            if !self.stop_gimbal() {
                return false;
            }
        } else {
            self.stop_gimbal();
        }
        self.backend.close();
        if self.gimbal.moving {
            log_warn!("Gimbal stop failed; the device is no longer available.");
        }
        log_warn!("Disconnected ({reason}).");
        self.gimbal.moving = false;
        self.shared.update_state(|s| {
            s.connected = false;
            s.xu = false;
            s.uvc = [UvcInfo::default(); UvcCtl::COUNT];
            s.status = Status::default();
            s.raw_status = [0; XU_LENGTH];
        });
        true
    }

    fn query_all_uvc(&mut self) {
        let mut infos = [UvcInfo::default(); UvcCtl::COUNT];
        for c in UvcCtl::ALL {
            if let Some(info) = self.backend.query_uvc(c) {
                infos[c.index()] = info;
            }
        }
        self.shared.update_state(|s| s.uvc = infos);
    }

    fn read_status(&mut self) {
        if !self.backend.has_xu() {
            return;
        }
        let Ok(raw) = self.backend.xu_get(protocol::SELECTOR_SIMPLE) else { return };
        let status = protocol::parse_status(&raw);
        let current = self.shared.snapshot();
        if current.raw_status != raw || current.status != status {
            self.shared.update_state(|s| {
                s.raw_status = raw;
                s.status = status;
            });
        }
    }

    /// Waits on the worker thread without starving the gimbal dead-man stop.
    fn pause(&mut self, length: Duration) {
        let until = Instant::now() + length;
        while self.shared.running() && !self.shared.switching() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
            if self.backend.is_open() {
                self.service_gimbal();
            }
        }
    }

    // ---- Extension-unit helpers ------------------------------------------------------

    fn send_simple(&mut self, what: &str, buf: &XuBuffer) -> bool {
        match self.backend.xu_set(protocol::SELECTOR_SIMPLE, buf) {
            Ok(()) => {
                log_info!("{what}: sent.");
                true
            }
            Err(e) => {
                log_error!("{what} failed: {e}");
                false
            }
        }
    }

    /// Sends a framed command. With `wait_for_reply`, first stops any gimbal movement,
    /// then polls the reply mailbox; returns the reply, or None on failure or no reply.
    /// Without it, returns an empty reply once the command was sent.
    fn send_framed(
        &mut self,
        what: &str,
        receiver: Receiver,
        command: u16,
        payload: &[u8],
        wait_for_reply: bool,
    ) -> Option<FrameReply> {
        if payload.len() > protocol::MAX_PAYLOAD {
            log_warn!("{what} rejected: payload is too large.");
            return None;
        }
        if wait_for_reply {
            self.shared.clear_gimbal_hold();
            if !self.shared.running() || !self.stop_gimbal() {
                return None;
            }
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.seq = 1;
        }
        let frame = protocol::build_frame(seq, receiver, command, payload);
        if let Err(e) = self.backend.xu_set(protocol::SELECTOR_FRAMED, &frame) {
            log_error!("{what} failed: {e}");
            return None;
        }
        if !wait_for_reply {
            return Some(FrameReply::default());
        }
        for _ in 0..REPLY_POLLS {
            {
                let queue = lock(&self.shared.queue);
                let _ = self
                    .shared
                    .queue_cv
                    .wait_timeout_while(queue, REPLY_POLL_INTERVAL, |_| {
                        self.shared.running() && !self.shared.switching()
                    });
            }
            if !self.shared.running() || self.shared.switching() {
                return None;
            }
            let Ok(raw) = self.backend.xu_get(protocol::SELECTOR_FRAMED) else { continue };
            if let Some(reply) = protocol::parse_frame(&raw) {
                if reply.seq == seq && reply.command == command {
                    log_info!("{what}: camera acknowledged.");
                    return Some(reply);
                }
            }
        }
        log_warn!("{what}: no reply from the camera.");
        None
    }

    // ---- Hold-to-move --------------------------------------------------------------------

    fn service_gimbal(&mut self) {
        let (pitch, yaw, refreshed) = {
            let g = lock(&self.shared.gimbal);
            (g.pitch, g.yaw, g.refreshed)
        };
        let now = Instant::now();
        let fresh = refreshed.is_some_and(|t| now - t < GIMBAL_DEAD_MAN);
        let want = !self.shared.switching() && (pitch != 0.0 || yaw != 0.0) && fresh;
        if want {
            if now - self.last_gimbal_send >= GIMBAL_RESEND || !self.gimbal.moving {
                self.last_gimbal_send = now;
                if !self.gimbal.moving {
                    log_info!("Gimbal moving (pitch {pitch:.0}\u{00B0}/s, yaw {yaw:.0}\u{00B0}/s).");
                }
                let payload = protocol::gimbal_speed_payload(pitch, yaw);
                let sent = self
                    .send_framed("Gimbal move", Receiver::Ai, protocol::cmd::GIMBAL_SPEED, &payload, false)
                    .is_some();
                self.gimbal.moving = sent || self.gimbal.moving;
            }
        } else if self.gimbal.moving {
            self.stop_gimbal();
        }
    }

    fn stop_gimbal(&mut self) -> bool {
        let was_moving = self.gimbal.moving;
        let mut motion = self.gimbal;
        let stopped = motion.stop(|| self.send_gimbal_stop());
        self.gimbal = motion;
        if was_moving && stopped {
            log_info!("Gimbal stopped.");
        }
        stopped
    }

    fn send_gimbal_stop(&mut self) -> bool {
        self.send_framed("Gimbal stop", Receiver::Ai, protocol::cmd::GIMBAL_SPEED, &[0u8; 12], false)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::demo::DemoCamera;

    fn wait_until(worker: &CameraWorker, mut done: impl FnMut(&CameraState) -> bool) -> CameraState {
        let start = Instant::now();
        loop {
            let s = worker.snapshot();
            if done(&s) || start.elapsed() > Duration::from_secs(5) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn connects_to_the_demo_camera_and_applies_commands() {
        let mut worker = CameraWorker::new();
        worker.set_anti_flicker_on_connect(POWER_LINE_60HZ);
        worker.start(Box::new(|| Box::new(DemoCamera::new())));
        let s = wait_until(&worker, |s| s.connected && s.status.valid);
        assert!(s.connected && s.xu);
        assert_eq!(s.variant, Variant::Lite);
        assert_eq!(s.uvc(UvcCtl::PowerLine).value, POWER_LINE_60HZ);

        worker.set_ai_mode(AiMode::Group);
        worker.set_hdr(true);
        worker.set_uvc(UvcCtl::Brightness, 500, false);
        worker.refresh_all();
        let s = wait_until(&worker, |s| s.status.hdr && s.status.ai_mode == Some(AiMode::Group));
        assert_eq!(s.status.ai_mode, Some(AiMode::Group));
        assert_eq!(s.uvc(UvcCtl::Brightness).value, 100, "clamped to the range");

        worker.set_sleep(true);
        let s = wait_until(&worker, |s| s.status.asleep);
        assert!(s.status.asleep);
        worker.stop();
    }

    #[test]
    fn framed_commands_wait_for_the_reply() {
        let mut worker = CameraWorker::new();
        worker.start(Box::new(|| Box::new(DemoCamera::new())));
        wait_until(&worker, |s| s.connected);
        worker.dev_framed(Receiver::Ai, protocol::cmd::TRACKING_SPEED, vec![2]);
        let s = wait_until(&worker, |s| !s.dev_output.is_empty());
        assert!(s.dev_output.contains("reply flags"), "{}", s.dev_output);
        worker.stop();
    }
}
