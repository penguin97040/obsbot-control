//! The user interface. All user-facing text is New Zealand English ("colour",
//! "centre", "licence" as a noun).
//!
//! One view owns everything the interface needs. A timer ticks about 60 times a
//! second: it copies the camera worker's state when it changes, takes new preview
//! frames, keeps hold-to-move alive, and runs the slower checks (camera use, window
//! size, pausing the preview while another window is active).

mod tabs;
pub mod theme;
mod titlebar;
mod widgets;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    canvas, deferred, div, fill, point, px, size, AnyElement, Bounds, Context, Corners, Div, FocusHandle, FontWeight,
    MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage, ScrollHandle, SharedString, Timer, Window,
};

use crate::camera::worker::{CameraState, CameraWorker, VariantSetting};
use crate::camera::UvcCtl;
use crate::camera_use::{self, CameraUse};
use crate::compact::{self, Policy, Switch, Transition};
use crate::logx;
use crate::preview::Preview;
use crate::preview_format::{self, PreviewFormat};
use crate::protocol::Model;
use crate::settings::Settings;
use crate::{log_info, safety, still_image};

use theme::*;
use widgets::*;

/// The window title. The Windows single-instance check looks for it.
pub const APP_TITLE: &str = "obsbot-control";

/// How often the interface checks for changes while something moves (the preview, a
/// held arrow or a dragged slider), and otherwise.
const TICK: Duration = Duration::from_millis(16);
const IDLE_TICK: Duration = Duration::from_millis(100);
const USE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const STEP_INTERVAL: Duration = Duration::from_millis(150);
/// Padding around the window's content, and the gap between the preview and the controls.
const PADDING: f32 = 12.0;
const GAP: f32 = 8.0;

/// The window's width without the invisible margins of a client-decorated Linux window.
fn content_width(window: &Window, frame: titlebar::Frame) -> f32 {
    f32::from(window.viewport_size().width) - frame.horizontal_inset()
}

/// How far through a transition that started at `start` is (1 or more when done).
fn progress(start: Instant) -> f32 {
    start.elapsed().as_secs_f32() / compact::TRANSITION.as_secs_f32()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SliderId {
    Uvc(UvcCtl),
    /// Pan or tilt, shown in degrees.
    Angle(UvcCtl),
    MoveSpeed,
}

/// Something whose on-screen position the interface needs after layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Anchor {
    Slider(SliderId),
    Menu(MenuKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuKind {
    Device,
    Size,
    Pixels,
    Rate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Menu {
    Dropdown(MenuKind),
    /// A preset's right-click menu, at the mouse position.
    Preset(usize, Point<Pixels>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Control,
    Image,
    Camera,
    Log,
    Developer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevField {
    Hex,
    Command,
    Payload,
}

/// The preview's formats, refreshed only when they may have changed.
#[derive(Default)]
struct FormatCache {
    generation: u64,
    requested: String,
    formats: Vec<PreviewFormat>,
    default_key: String,
    def: Option<PreviewFormat>,
    cur: Option<PreviewFormat>,
}

pub struct AppView {
    settings: Settings,
    worker: CameraWorker,
    state: CameraState,
    state_generation: u64,

    preview: Preview,
    preview_image: Option<Arc<RenderImage>>,
    /// Images replaced last tick, released from the GPU on the next one.
    retired_images: Vec<Arc<RenderImage>>,
    /// A blurred copy of the last frame and the shape of that frame.
    still: Option<(Arc<RenderImage>, (u32, u32))>,
    /// Stops a failed preview restarting every tick.
    preview_attempt_path: String,
    /// The chosen preview format; empty means the default. Not saved, so the app
    /// always opens at the default format.
    preview_format_key: String,
    preview_format_device: String,
    formats: FormatCache,

    compact: bool,
    /// The preview is paused because another window is active.
    paused: bool,
    /// The view changed: start the transition as soon as there is a window to hand.
    resize_pending: bool,
    /// The running switch between full and compact view, and when it started.
    transition: Option<(Instant, Transition)>,
    /// A display-frame callback is queued to move the transition on.
    frame_loop: bool,
    title_state: titlebar::State,
    /// How much of the preview is shown when no transition runs (see `shown`).
    rest_shown: f32,
    /// The window width to go back to in full view.
    full_width: f32,
    policy: Policy,
    camera_use: CameraUse,
    last_use_check: Option<Instant>,

    tab: Tab,
    /// The arrow-pad direction being held (dx: +1 right, dy: +1 up).
    held: Option<(i32, i32)>,
    last_step: Instant,
    drag: Option<SliderId>,
    anchors: Rc<RefCell<HashMap<Anchor, Bounds<Pixels>>>>,
    menu: Option<Menu>,

    log_generation: u64,
    log_entries: Vec<logx::Entry>,
    log_scroll: ScrollHandle,
    log_follow: bool,

    dev_selector: u8,
    dev_hex: String,
    dev_receiver: usize,
    dev_command: String,
    dev_payload: String,
    dev_focus: [FocusHandle; 3],

    shut_down: bool,
    /// Using the demo camera: nothing is saved.
    demo: bool,
}

impl AppView {
    pub fn new(settings: Settings, demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut worker = CameraWorker::new();
        worker.set_variant_setting(VariantSetting::from_index(settings.variant));
        worker.set_anti_flicker_on_connect(settings.anti_flicker);
        worker.select_device(&settings.device_path);
        if demo {
            worker.start(Box::new(|| Box::new(crate::camera::demo::DemoCamera::new())));
        } else {
            worker.start(Box::new(crate::camera::platform_backend));
        }
        let full_width = settings.window_w.max(640) as f32;
        let mut view = AppView {
            settings,
            worker,
            state: CameraState::default(),
            state_generation: 0,
            preview: Preview::default(),
            preview_image: None,
            retired_images: Vec::new(),
            still: None,
            preview_attempt_path: String::new(),
            preview_format_key: String::new(),
            preview_format_device: String::new(),
            formats: FormatCache { generation: u64::MAX, ..Default::default() },
            compact: false,
            paused: false,
            resize_pending: false,
            transition: None,
            frame_loop: false,
            title_state: titlebar::State::default(),
            rest_shown: 1.0,
            full_width,
            policy: Policy::default(),
            camera_use: CameraUse::default(),
            last_use_check: None,
            tab: Tab::Control,
            held: None,
            last_step: Instant::now(),
            drag: None,
            anchors: Rc::new(RefCell::new(HashMap::new())),
            menu: None,
            log_generation: u64::MAX,
            log_entries: Vec::new(),
            log_scroll: ScrollHandle::new(),
            log_follow: true,
            dev_selector: 6,
            dev_hex: String::new(),
            dev_receiver: 0,
            dev_command: String::new(),
            dev_payload: String::new(),
            dev_focus: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            shut_down: false,
            demo,
        };
        // Starts compact if another app is already using the camera.
        view.check_camera_use();
        view.start_transition(window, false);

        cx.spawn_in(window, async move |this, cx| {
            let mut delay = TICK;
            loop {
                Timer::after(delay).await;
                match this.update_in(cx, |view, window, cx| {
                    view.tick(window, cx);
                    view.busy()
                }) {
                    Ok(busy) => delay = if busy { TICK } else { IDLE_TICK },
                    Err(_) => break,
                }
            }
        })
        .detach();
        view
    }

    /// Saves settings and stops the preview and camera threads. Safe to call twice.
    pub fn shutdown(&mut self, window: &Window) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        self.held = None;
        self.worker.release_gimbal();
        self.preview.stop();
        self.worker.stop();
        if !window.is_maximized() {
            let viewport = window.viewport_size();
            // Mid-switch the window is part-way between widths: keep the full-view one.
            let width =
                if self.compact || self.transition.is_some() { self.full_width } else { f32::from(viewport.width) };
            self.settings.window_w = width.round() as i32;
            self.settings.window_h = f32::from(viewport.height).round() as i32;
        }
        self.save_settings();
    }

    // ---- The tick ---------------------------------------------------------------------

    /// True while something on screen moves, so the tick should run at full speed.
    fn busy(&self) -> bool {
        self.preview.running() || self.held.is_some() || self.drag.is_some() || self.resize_pending
    }

    /// Saves the settings, except with the demo camera, which must never change the
    /// real camera's settings (such as which camera was last used).
    fn save_settings(&self) {
        if !self.demo {
            self.settings.save();
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shut_down {
            return;
        }
        let mut changed = false;
        for image in self.retired_images.drain(..) {
            let _ = window.drop_image(image);
        }

        let generation = self.worker.generation();
        if generation != self.state_generation {
            self.state_generation = generation;
            self.state = self.worker.snapshot();
            changed = true;
        }
        if self.tab == Tab::Log {
            let log_generation = logx::generation();
            if log_generation != self.log_generation {
                self.log_generation = log_generation;
                self.log_entries = logx::snapshot();
                if self.log_follow && !self.log_entries.is_empty() {
                    self.log_scroll.scroll_to_item(self.log_entries.len() - 1);
                }
                changed = true;
            }
        }

        if self.state.connected && self.state.device.path != self.settings.device_path {
            self.settings.device_path = self.state.device.path.clone();
        }
        if self.last_use_check.is_none_or(|t| t.elapsed() >= USE_CHECK_INTERVAL) {
            let was = (self.compact, self.camera_use.clone());
            self.check_camera_use();
            changed |= was != (self.compact, self.camera_use.clone());
        }
        let was_paused = self.paused;
        self.begin_transition(window, cx);
        // Display frames drive the switch, but a hidden window may get none; finish it
        // here then, so the camera is released and the window size settles.
        if self.transition.is_some_and(|(start, _)| progress(start) >= 2.0) {
            self.step_transition(window);
            changed = true;
        }
        changed |= self.manage_preview(window);
        changed |= was_paused != self.paused;

        if let Some(frame) = self.preview.take_frame() {
            if let Some(image) = render_image(frame.width, frame.height, frame.bgra) {
                if let Some(old) = self.preview_image.replace(image) {
                    self.retired_images.push(old);
                }
                changed = true;
            }
        }

        if !window.is_window_active() && self.held.is_some() {
            self.release_hold();
            changed = true;
        }
        self.service_hold();

        if changed {
            cx.notify();
        }
    }

    // ---- Compact view ------------------------------------------------------------------

    fn check_camera_use(&mut self) {
        self.last_use_check = Some(Instant::now());
        if !self.settings.auto_compact {
            return;
        }
        self.camera_use =
            camera_use::query(if self.state.connected { &self.state.device.path } else { "" }, self.preview.running());
        match self.policy.on_detected(self.camera_use.in_use, self.compact) {
            Switch::ToCompact => {
                log_info!("{} is using the camera: switching to compact view.", self.camera_use_name());
                self.set_compact(true);
            }
            Switch::ToFull => {
                log_info!("The other app has finished with the camera: switching to full view.");
                self.set_compact(false);
            }
            Switch::None => {}
        }
    }

    fn camera_use_name(&self) -> String {
        if self.camera_use.app_name.is_empty() {
            "Another app".to_string()
        } else {
            self.camera_use.app_name.clone()
        }
    }

    fn set_compact(&mut self, on: bool) {
        if on != self.compact {
            self.compact = on;
            self.resize_pending = true;
        }
    }

    /// The window width full view returns to, with `inset` the Linux margins.
    fn full_view_width(&self, inset: f32) -> f32 {
        self.full_width.max(COMPACT_WIDTH + inset + 200.0)
    }

    /// How much of the preview is shown: 1 in full view, 0 in compact view, and in
    /// between while switching.
    fn shown(&self) -> f32 {
        match self.transition {
            Some((start, t)) => t.at(progress(start)).0,
            None => self.rest_shown,
        }
    }

    /// Starts switching to the current view: the window narrows to the controls, or
    /// gets back the width it had in full view, while the preview slides away or back.
    /// Without `animate` (at start-up) it jumps straight there.
    fn start_transition(&mut self, window: &mut Window, animate: bool) {
        if !self.resize_pending {
            return;
        }
        self.resize_pending = false;
        let inset = titlebar::Frame::of(window).horizontal_inset();
        let now = f32::from(window.viewport_size().width);
        let widths = (!window.is_maximized() && !window.is_fullscreen()).then(|| {
            let target = if self.compact {
                // Mid-way through a switch, the widths are already known: the full width is kept and
                // the compact one is the running transition's own, so a reversal ends where it planned.
                match self.transition.and_then(|(_, t)| t.ends) {
                    Some(ends) => ends.0,
                    None => {
                        if self.transition.is_none() {
                            self.full_width = now.max(COMPACT_WIDTH + inset + 1.0);
                        }
                        COMPACT_WIDTH + inset
                    }
                }
            } else {
                self.full_view_width(inset)
            };
            (now, target)
        });
        let to_shown = if self.compact { 0.0 } else { 1.0 };
        if !animate {
            self.transition = None;
            self.rest_shown = to_shown;
            if let Some((_, width)) = widths {
                window.resize(size(px(width), window.viewport_size().height));
            }
            return;
        }
        let ends = widths.map(|(now, target)| match self.transition.and_then(|(_, t)| t.ends) {
            Some(ends) => ends,
            None if self.compact => (target, now),
            None => (now, target),
        });
        let transition = Transition { from_shown: self.shown(), to_shown, widths, ends };
        self.transition = Some((Instant::now(), transition));
    }

    /// Starts a pending view switch and drives it from display frames, so it moves at
    /// the screen's refresh rate and costs nothing once it ends. (A timer can't do
    /// this: Windows timers tick every 15.6 ms, so a 16 ms one runs at about 30 fps.)
    fn begin_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_transition(window, true);
        if self.transition.is_some() && !self.frame_loop {
            self.frame_loop = true;
            self.next_transition_frame(window, cx);
        }
    }

    fn next_transition_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |this, window, cx| {
            this.step_transition(window);
            cx.notify();
            if this.transition.is_some() {
                this.next_transition_frame(window, cx);
            } else {
                this.frame_loop = false;
            }
        });
    }

    /// Moves the running transition on to the current time.
    fn step_transition(&mut self, window: &mut Window) {
        let Some((start, transition)) = self.transition else { return };
        let t = progress(start);
        if let (_, Some(width)) = transition.at(t) {
            if !window.is_maximized() && !window.is_fullscreen() {
                window.resize(size(px(width.round()), window.viewport_size().height));
            }
        }
        if t >= 1.0 {
            self.transition = None;
            self.rest_shown = transition.to_shown;
        }
    }

    // ---- Preview ------------------------------------------------------------------------

    /// Starts and stops the preview. Returns true when what is shown may have changed.
    fn manage_preview(&mut self, window: &mut Window) -> bool {
        // In the background the preview lets go of the camera, so another app (Discord,
        // Zoom) can turn it on; the camera-use check then notices and goes compact.
        self.paused = self.settings.pause_in_background && !window.is_window_active();
        // It keeps running while it slides away, so the picture doesn't vanish first.
        let shown = !self.compact || self.transition.is_some();
        let want = self.settings.preview_enabled && self.state.connected && shown && !self.paused;
        if !want {
            let mut changed = false;
            if self.preview.running() {
                self.preview.stop();
                self.keep_still();
                changed = true;
            }
            if !self.settings.preview_enabled && self.still.is_some() {
                self.retire_still();
                changed = true;
            }
            self.preview_attempt_path.clear();
            return changed;
        }
        // A different camera starts again at its default format.
        if self.state.device.path != self.preview_format_device {
            self.preview_format_device = self.state.device.path.clone();
            self.preview_format_key.clear();
            self.retire_still();
        }
        let same_request = self.preview.device_path() == self.state.device.path
            && self.preview.requested_format() == self.preview_format_key;
        if self.preview.running() && same_request {
            return false;
        }
        if !self.preview.running() && same_request && self.preview_attempt_path == self.state.device.path {
            return false; // it failed: wait for Retry
        }
        if self.preview.running() {
            self.preview.stop();
        }
        self.keep_still();
        self.preview_attempt_path = self.state.device.path.clone();
        let path = self.state.device.path.clone();
        let key = self.preview_format_key.clone();
        self.preview.start(&path, &key);
        true
    }

    /// Turns the last frame into the blurred still shown while paused or restarting.
    fn keep_still(&mut self) {
        let Some(image) = self.preview_image.take() else { return };
        let shape = image.size(0);
        let (w, h) = (shape.width.0 as u32, shape.height.0 as u32);
        if let Some(bytes) = image.as_bytes(0) {
            if let Some((pixels, tw, th)) = still_image::blurred_thumbnail(bytes, w, h, 48) {
                if let Some(still) = render_image(tw, th, pixels) {
                    self.retire_still();
                    self.still = Some((still, (w, h)));
                }
            }
        }
        self.retired_images.push(image);
    }

    fn retire_still(&mut self) {
        if let Some((image, _)) = self.still.take() {
            self.retired_images.push(image);
        }
    }

    fn refresh_formats(&mut self) {
        let generation = self.preview.format_generation();
        if generation == self.formats.generation && self.preview_format_key == self.formats.requested {
            return;
        }
        let c = &mut self.formats;
        c.generation = generation;
        c.requested = self.preview_format_key.clone();
        c.formats = self.preview.formats();
        c.default_key = self.preview.default_format_key();
        c.def = preview_format::find(&c.formats, &c.default_key).cloned();
        // What's showing now: the running format, else the one asked for, else the default.
        let wanted = if c.requested.is_empty() { c.default_key.clone() } else { c.requested.clone() };
        c.cur = preview_format::find(&c.formats, &self.preview.current_format_key())
            .or_else(|| preview_format::find(&c.formats, &wanted))
            .cloned()
            .or_else(|| c.def.clone());
    }

    fn choose_format(&mut self, width: u32, height: u32, fourcc: &str, fps: i32) {
        let fallback = self.formats.def.as_ref().map(|d| d.fourcc.clone()).unwrap_or_default();
        let key = preview_format::choose(&self.formats.formats, width, height, fourcc, fps, &fallback);
        if key.is_empty() {
            return;
        }
        self.preview_format_key = if key == self.formats.default_key { String::new() } else { key };
        self.preview_attempt_path.clear();
    }

    // ---- Hold-to-move ----------------------------------------------------------------------

    fn start_hold(&mut self, dx: i32, dy: i32) {
        self.held = Some((dx, dy));
        self.last_step = Instant::now() - STEP_INTERVAL;
        self.service_hold();
    }

    fn release_hold(&mut self) {
        if self.held.take().is_some() {
            self.worker.release_gimbal();
        }
    }

    fn service_hold(&mut self) {
        let Some((dx, dy)) = self.held else { return };
        if !self.state.connected {
            return;
        }
        let smooth = self.settings.move_method == crate::settings::MOVE_GIMBAL_SPEED;
        // move_signs knows which axis each method runs the opposite way on (checked on a
        // Tiny 2 Lite) and applies the invert switches on top.
        let sign = safety::move_signs(smooth, dx, dy, self.settings.invert_horizontal, self.settings.invert_vertical);
        if smooth {
            let speed = self.settings.move_speed;
            self.worker.hold_gimbal_velocity(sign.tilt as f32 * speed, sign.pan as f32 * speed);
        } else if self.last_step.elapsed() >= STEP_INTERVAL {
            self.last_step = Instant::now();
            for (c, dir) in [(UvcCtl::Pan, sign.pan), (UvcCtl::Tilt, sign.tilt)] {
                let u = *self.state.uvc(c);
                if dir != 0 && u.supported {
                    self.worker.set_uvc(c, safety::step_value(u.value, u.min, u.max, u.step, dir), false);
                }
            }
        }
    }

    // ---- Sliders ----------------------------------------------------------------------------

    /// Where a slider's grab sits (0..1) and whether it can be used.
    fn slider_position(&self, id: SliderId) -> (f32, bool) {
        let fraction =
            |value: f64, min: f64, max: f64| if max > min { ((value - min) / (max - min)) as f32 } else { 0.0 };
        match id {
            SliderId::Uvc(c) | SliderId::Angle(c) => {
                let u = self.state.uvc(c);
                (
                    fraction(u.value as f64, u.min as f64, u.max as f64),
                    u.supported && !u.is_auto && self.state.connected,
                )
            }
            SliderId::MoveSpeed => (
                fraction(self.settings.move_speed as f64, 5.0, 90.0),
                self.settings.move_method == crate::settings::MOVE_GIMBAL_SPEED,
            ),
        }
    }

    /// Moves a slider to the mouse position.
    fn drag_slider(&mut self, id: SliderId, mouse: Point<Pixels>) {
        let Some(bounds) = self.anchors.borrow().get(&Anchor::Slider(id)).copied() else { return };
        let width = f32::from(bounds.size.width);
        if width <= 0.0 {
            return;
        }
        let t = (f32::from(mouse.x - bounds.origin.x) / width).clamp(0.0, 1.0) as f64;
        match id {
            SliderId::Uvc(c) => {
                let u = *self.state.uvc(c);
                let step = u.step.max(1) as f64;
                let raw = u.min as f64 + t * (u.max as f64 - u.min as f64);
                let value = (u.min as f64 + ((raw - u.min as f64) / step).round() * step).round() as i64;
                let value = value.clamp(u.min as i64, u.max as i64) as i32;
                if value != u.value {
                    self.worker.set_uvc(c, value, false);
                }
            }
            SliderId::Angle(c) => {
                let u = *self.state.uvc(c);
                let per_degree = angle_scale(u.min, u.max);
                let degrees = ((u.min as f64 + t * (u.max as f64 - u.min as f64)) / per_degree).round();
                let value = safety::angle_value(degrees, per_degree, u.min, u.max);
                if value != u.value {
                    self.worker.set_uvc(c, value, false);
                }
            }
            SliderId::MoveSpeed => {
                self.settings.move_speed = safety::move_speed((5.0 + t * 85.0).round() as f32);
            }
        }
        // Show the new value straight away; the next tick copies the rest.
        if let SliderId::Uvc(c) | SliderId::Angle(c) = id {
            self.state.uvc[c.index()] = self.worker.uvc(c);
        }
    }

    fn slider(&self, id: SliderId, cx: &mut Context<Self>) -> impl IntoElement {
        let (t, enabled) = self.slider_position(id);
        let active = self.drag == Some(id);
        let anchors = self.anchors.clone();
        let mut el = div()
            .id(SharedString::from(format!("slider-{id:?}")))
            .flex_1()
            .min_w(px(48.0))
            .h(px(CONTROL_HEIGHT))
            .px(px(7.0))
            .flex()
            .items_center()
            .child(
                div().relative().w_full().child(slider_visual(t, active, enabled)).child(
                    canvas(
                        move |bounds, _, _| {
                            anchors.borrow_mut().insert(Anchor::Slider(id), bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
            );
        if enabled {
            el = el.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        // Double-click puts a camera setting back to its default.
                        if let SliderId::Uvc(c) = id {
                            let def = this.state.uvc(c).def;
                            this.worker.set_uvc(c, def, false);
                            this.drag = None;
                            cx.notify();
                            return;
                        }
                    }
                    this.drag = Some(id);
                    this.drag_slider(id, event.position);
                    cx.notify();
                }),
            );
            if let SliderId::Uvc(c) = id {
                let def = self.state.uvc(c).def;
                el = el.tooltip(tip(format!("Double-click to reset (default {def}).")));
            }
        }
        el
    }

    /// A UVC control row: label, slider, value and (optionally) an Auto switch.
    fn uvc_row(
        &self,
        c: UvcCtl,
        label: &str,
        auto_column: bool,
        unit_slot: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let u = *self.state.uvc(c);
        let mut row = div().flex().items_center().min_h(px(CONTROL_HEIGHT)).child(label_cell(
            SharedString::from(format!("label-{c:?}")),
            label,
            LABEL_WIDTH,
            None,
        ));
        if !u.supported {
            return row.child(note("Not available")).into_any_element();
        }
        row = row
            .child(self.slider(SliderId::Uvc(c), cx))
            .child(value_text(u.value.to_string(), "", unit_slot).when(u.is_auto, |d| d.opacity(0.55)));
        if auto_column {
            let switch = if u.can_auto {
                toggle_switch(
                    SharedString::from(format!("auto-{c:?}")),
                    Some("Auto"),
                    u.is_auto,
                    self.state.connected,
                    cx.listener(move |this, on: &bool, _, _| {
                        let value = this.state.uvc(c).value;
                        this.worker.set_uvc(c, value, *on);
                    }),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            };
            row = row.child(div().flex_none().w(px(76.0)).pl(px(10.0)).child(switch));
        }
        row.into_any_element()
    }

    /// Pan or tilt in degrees (large UVC ranges are arc-seconds).
    fn angle_row(&self, c: UvcCtl, label: &str, cx: &mut Context<Self>) -> AnyElement {
        let u = *self.state.uvc(c);
        let row = div().flex().items_center().min_h(px(CONTROL_HEIGHT)).child(label_cell(
            SharedString::from(format!("label-{c:?}")),
            label,
            LABEL_WIDTH,
            None,
        ));
        if !u.supported {
            return row.child(note("Not available")).into_any_element();
        }
        let degrees = u.value as f64 / angle_scale(u.min, u.max);
        row.child(self.slider(SliderId::Angle(c), cx))
            .child(value_text(format!("{degrees:.0}"), "\u{00B0}", true))
            .into_any_element()
    }

    // ---- Menus -------------------------------------------------------------------------------

    fn toggle_menu(&mut self, kind: MenuKind) {
        self.menu = if self.menu == Some(Menu::Dropdown(kind)) { None } else { Some(Menu::Dropdown(kind)) };
    }

    /// A dropdown's button, which also records where it is so its list can open below it.
    fn dropdown(
        &self,
        kind: MenuKind,
        label: String,
        width: Option<f32>,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.menu == Some(Menu::Dropdown(kind));
        let anchors = self.anchors.clone();
        let button = dropdown_button(SharedString::from(format!("dropdown-{kind:?}")), label, open, enabled)
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_menu(kind);
                    cx.notify();
                }))
            })
            .when_some(width, |d, w| d.w(px(w)));
        div().relative().flex_none().child(button).child(
            canvas(
                move |bounds, _, _| {
                    anchors.borrow_mut().insert(Anchor::Menu(kind), bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }

    /// The open dropdown list or context menu, drawn above everything else.
    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu?;
        let (position, min_width, items): (Point<Pixels>, Pixels, Vec<AnyElement>) = match menu {
            Menu::Dropdown(kind) => {
                let bounds = self.anchors.borrow().get(&Anchor::Menu(kind)).copied()?;
                (bounds.bottom_left() + point(px(0.0), px(2.0)), bounds.size.width, self.menu_items(kind, cx))
            }
            Menu::Preset(i, position) => (position, px(240.0), self.preset_menu_items(i, cx)),
        };
        let panel = menu_panel(min_width).id("menu").overflow_y_scroll().children(items).on_mouse_down_out(
            cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }),
        );
        Some(
            deferred(gpui::anchored().position(position).snap_to_window_with_margin(px(8.0)).child(panel))
                .with_priority(1)
                .into_any_element(),
        )
    }

    fn menu_items(&mut self, kind: MenuKind, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items = Vec::new();
        let close = |this: &mut AppView, cx: &mut Context<AppView>| {
            this.menu = None;
            cx.notify();
        };
        match kind {
            MenuKind::Device => {
                for (i, d) in self.state.available.iter().enumerate() {
                    let path = d.path.clone();
                    let selected = d.path == self.state.device.path;
                    items.push(
                        menu_item(
                            ("device", i),
                            d.name.clone(),
                            selected,
                            true,
                            Box::new(cx.listener(move |this, _, _, cx| {
                                this.settings.device_path = path.clone();
                                this.worker.select_device(&path);
                                close(this, cx);
                            })),
                        )
                        .into_any_element(),
                    );
                }
            }
            MenuKind::Size | MenuKind::Pixels | MenuKind::Rate => {
                self.refresh_formats();
                let Some(cur) = self.formats.cur.clone() else { return items };
                let def = self.formats.def.clone();
                let def_size = def.as_ref().is_some_and(|d| d.width == cur.width && d.height == cur.height);
                let label =
                    |text: String, is_default: bool| if is_default { format!("{text}  (default)") } else { text };
                match kind {
                    MenuKind::Size => {
                        for (i, (w, h)) in preview_format::resolutions(&self.formats.formats).into_iter().enumerate() {
                            let is_default = def.as_ref().is_some_and(|d| d.width == w && d.height == h);
                            let (fourcc, fps) = (cur.fourcc.clone(), cur.rounded_fps());
                            items.push(
                                menu_item(
                                    ("size", i),
                                    label(PreviewFormat::size_label(w, h), is_default),
                                    w == cur.width && h == cur.height,
                                    true,
                                    Box::new(cx.listener(move |this, _, _, cx| {
                                        this.choose_format(w, h, &fourcc, fps);
                                        close(this, cx);
                                    })),
                                )
                                .into_any_element(),
                            );
                        }
                    }
                    MenuKind::Pixels => {
                        for (i, fourcc) in preview_format::pixel_formats(&self.formats.formats, cur.width, cur.height)
                            .into_iter()
                            .enumerate()
                        {
                            let is_default = def_size && def.as_ref().is_some_and(|d| d.fourcc == fourcc);
                            let (w, h, fps) = (cur.width, cur.height, cur.rounded_fps());
                            let chosen = fourcc.clone();
                            items.push(
                                menu_item(
                                    ("pixels", i),
                                    label(fourcc.clone(), is_default),
                                    fourcc == cur.fourcc,
                                    true,
                                    Box::new(cx.listener(move |this, _, _, cx| {
                                        this.choose_format(w, h, &chosen, fps);
                                        close(this, cx);
                                    })),
                                )
                                .into_any_element(),
                            );
                        }
                    }
                    MenuKind::Rate => {
                        for (i, fps) in
                            preview_format::frame_rates(&self.formats.formats, cur.width, cur.height, &cur.fourcc)
                                .into_iter()
                                .enumerate()
                        {
                            let is_default = def_size
                                && def.as_ref().is_some_and(|d| d.fourcc == cur.fourcc && d.rounded_fps() == fps);
                            let (w, h, fourcc) = (cur.width, cur.height, cur.fourcc.clone());
                            items.push(
                                menu_item(
                                    ("rate", i),
                                    label(PreviewFormat::fps_label(fps), is_default),
                                    fps == cur.rounded_fps(),
                                    true,
                                    Box::new(cx.listener(move |this, _, _, cx| {
                                        this.choose_format(w, h, &fourcc, fps);
                                        close(this, cx);
                                    })),
                                )
                                .into_any_element(),
                            );
                        }
                    }
                    MenuKind::Device => {}
                }
            }
        }
        items
    }

    // ---- Header and preview ---------------------------------------------------------------------

    /// The header row, which is also the window's title bar. Below `HEADER_WIDE` (and
    /// in compact view) it drops the model name, shortens the status to its dot and the
    /// sleep button to one word, and moves the camera picker to a row of its own.
    fn render_header(&mut self, frame: titlebar::Frame, window: &Window, cx: &mut Context<Self>) -> Div {
        let s = &self.state;
        let asleep = s.status.valid && s.status.asleep;
        let (text, colour) = if !s.connected {
            ("Not connected", bad())
        } else if asleep {
            ("Asleep", warn())
        } else {
            ("Connected", good())
        };
        let picker = s.connected && s.available.len() > 1;
        // The camera picker (200 px and a gap) needs that much more room to stay wide.
        let wide_from = HEADER_WIDE + if picker { 208.0 } else { 0.0 };
        let narrow = self.compact || content_width(window, frame) < wide_from;
        let model = if s.connected && !narrow { Some(s.device.model) } else { None };
        let connected = s.connected;
        let xu = s.xu;

        let views = ["Full", "Compact"];
        let view_tips = [
            "Show the preview beside the controls.",
            "Hide the preview and narrow the window to the controls. Turns on by itself while another app (Zoom, Discord, Teams...) is using the camera, and off again when it stops.",
        ];
        // On the left, like the controls: the window's left edge stays put when it
        // narrows or widens, so the switch stays under the mouse (COMPACT_WIDTH leaves
        // room for it). Occluded, so clicks here never start a window drag.
        let view_switch = div().id("view-switch").flex_none().w(px(140.0)).flex().occlude().child(segmented(
            "view",
            &views,
            &view_tips,
            Some(self.compact as usize),
            true,
            cx.listener(|this, i: &usize, window, cx| {
                let compact = *i == 1;
                if compact != this.compact {
                    this.policy.on_manual_toggle();
                    this.set_compact(compact);
                    // Straight away, not on the next tick, so the click feels instant
                    // and the preview starts while it slides in.
                    this.begin_transition(window, cx);
                    this.manage_preview(window);
                    cx.notify();
                }
            }),
        ));

        let left = div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(10.0))
            .min_w(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .text_color(accent_main())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(WORDMARK_SIZE))
                    .child("obsbot-control"),
            )
            // Before the status, whose width changes, so the switch sits in the same
            // place in both views.
            .child(view_switch)
            .child(if narrow {
                // Occluded so Windows doesn't treat it as part of the drag area, which
                // would stop its tooltip showing.
                div().id("status").occlude().child(status_dot(colour)).tooltip(tip(text)).into_any_element()
            } else {
                status_pill(text, colour, window.scale_factor()).into_any_element()
            })
            .when_some(model, |d, m: Model| {
                d.child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_color(text_dim())
                        .child(m.name()),
                )
            });

        let sleep_tip = if asleep { "Wake the camera." } else { "Put the camera to sleep." };
        // Occluded, so clicks here never start a window drag.
        let right = div()
            .id("header-controls")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .occlude()
            .when(picker && !narrow, |d| d.child(self.device_picker(cx)))
            .when(connected, |d| {
                let toggle = move |this: &mut Self| this.worker.set_sleep(!asleep);
                // Narrow headers get a moon icon (amber while asleep) instead of words.
                d.child(if narrow {
                    icon_button(
                        "sleep",
                        moon(if asleep { warn() } else { theme::text() }),
                        xu,
                        Some(Box::new(cx.listener(move |this, _, _, _| toggle(this)))),
                    )
                    .tooltip(tip(sleep_tip))
                } else {
                    button(
                        "sleep",
                        if asleep { "Wake camera" } else { "Sleep camera" },
                        xu,
                        Some(Box::new(cx.listener(move |this, _, _, _| toggle(this)))),
                    )
                })
            });

        let bar = titlebar::drag_area(div().id("header"), frame, &self.title_state, window)
            .flex_none()
            .h(px(titlebar::BAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(PADDING))
            .pr(px(if frame.own_bar { 0.0 } else { 12.0 }))
            .child(left)
            .child(right)
            .when(frame.own_bar, |d| {
                d.child(titlebar::window_buttons(
                    window,
                    frame,
                    cx.listener(|this, _, window, _| {
                        this.shutdown(window);
                        window.remove_window();
                    }),
                ))
            });

        div()
            .flex()
            .flex_col()
            .child(bar)
            .when(picker && narrow, |d| d.child(div().flex().px(px(PADDING)).pb(px(4.0)).child(self.device_picker(cx))))
    }

    fn device_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let name = self.state.device.name.clone();
        self.dropdown(MenuKind::Device, name, Some(200.0), true, cx).into_any_element()
    }

    fn render_preview_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_formats();
        let mut bar = div().flex().items_center().gap(px(6.0)).min_h(px(CONTROL_HEIGHT));
        let pickers = self.state.connected && self.settings.preview_enabled;
        match self.formats.cur.clone().filter(|_| pickers) {
            Some(cur) => {
                bar = bar
                    .child(div().id("size-tip").child(self.dropdown(MenuKind::Size, PreviewFormat::size_label(cur.width, cur.height), None, true, cx))
                        .tooltip(tip("Preview resolution. Larger sizes use more processor time.")))
                    .child(div().id("pixels-tip").child(self.dropdown(MenuKind::Pixels, cur.fourcc.clone(), None, true, cx))
                        .tooltip(tip("How the camera sends each frame. NV12 and YUYV are uncompressed; MJPG is compressed and allows higher frame rates at large sizes.")))
                    .child(div().id("rate-tip").child(self.dropdown(MenuKind::Rate, PreviewFormat::fps_label(cur.rounded_fps()), None, true, cx))
                        .tooltip(tip("Preview frame rate.")));
            }
            None => bar = bar.child(div().text_color(text_dim()).child("Preview")),
        }
        bar.child(div().flex_1())
            .child(
                div()
                    .id("live-preview")
                    .child(toggle_switch(
                        "preview-switch",
                        Some("Live preview"),
                        self.settings.preview_enabled,
                        true,
                        cx.listener(|this, on: &bool, _, cx| {
                            this.settings.preview_enabled = *on;
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .tooltip(tip("Turn this off if you want another app (Teams, Zoom, OBS) to use the camera while this window is open. Controls keep working without the preview.")),
            )
    }

    fn render_preview_area(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let running = self.preview.running();
        let error = self.preview.error();
        let image = if running { self.preview_image.clone() } else { None };
        let restarting = running && error.is_empty() && image.is_none();
        let still = self
            .still
            .clone()
            .filter(|_| s.connected && self.settings.preview_enabled && (self.paused || restarting) && image.is_none());

        // Painted directly, so the picture is letterboxed to the real size of the area.
        let picture = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let (data, shape, dim) = match (&image, &still) {
                    (Some(img), _) => {
                        let s = img.size(0);
                        (Some(img.clone()), (s.width.0 as f32, s.height.0 as f32), false)
                    }
                    (None, Some((img, shape))) => (Some(img.clone()), (shape.0 as f32, shape.1 as f32), true),
                    _ => (None, (0.0, 0.0), false),
                };
                let area = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let rect = if data.is_some() && shape.0 > 0.0 && shape.1 > 0.0 {
                    let scale = (area.0 / shape.0).min(area.1 / shape.1);
                    let (w, h) = (shape.0 * scale, shape.1 * scale);
                    Bounds::new(
                        bounds.origin + point(px((area.0 - w) * 0.5), px((area.1 - h) * 0.5)),
                        size(px(w), px(h)),
                    )
                } else {
                    bounds
                };
                if let Some(data) = data {
                    let _ = window.paint_image(rect, Corners::default(), data, 0, false);
                    if dim {
                        window.paint_quad(fill(rect, {
                            let mut c = bg();
                            c.a = 0.55;
                            c
                        }));
                    }
                }
                let colour = if image.is_some() { accent_dim() } else { accent(0.15) };
                paint_brackets(window, rect, colour);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let centred = |content: AnyElement| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .p(px(24.0))
                .child(content)
        };
        let message: Option<AnyElement> = if !s.connected {
            Some(note("Waiting for an OBSBOT camera...").text_size(px(TEXT_SIZE)).into_any_element())
        } else if !self.settings.preview_enabled {
            Some(note("Preview is off.").text_size(px(TEXT_SIZE)).into_any_element())
        } else if self.paused {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().text_color(text()).child("Preview paused"))
                    .child(note("It resumes when you come back to this window."))
                    .into_any_element(),
            )
        } else if !error.is_empty() && !running {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .max_w(px(420.0))
                    .child(div().text_color(warn()).text_center().child(error))
                    .child(button(
                        "retry",
                        "Retry preview",
                        true,
                        Some(Box::new(cx.listener(|this, _, _, cx| {
                            // Go back to the default format in case the chosen one was the problem.
                            this.preview_format_key.clear();
                            this.preview_attempt_path.clear();
                            cx.notify();
                        }))),
                    ))
                    .into_any_element(),
            )
        } else if self.preview_image.is_none() || !running {
            Some(note("Starting preview...").text_size(px(TEXT_SIZE)).into_any_element())
        } else {
            None
        };

        div().relative().flex_1().min_h(px(120.0)).child(picture).when_some(message, |d, m| d.child(centred(m)))
    }

    fn render_tabs_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs =
            vec![(Tab::Control, "Control"), (Tab::Image, "Image"), (Tab::Camera, "Camera"), (Tab::Log, "Log")];
        if self.settings.show_developer {
            tabs.push((Tab::Developer, "Developer"));
        }
        // Underlined tabs: the selected one is bright with an accent bar along the line.
        let mut bar = div().flex().gap(px(2.0)).border_b_1().border_color(border());
        for (tab, label) in tabs {
            let selected = self.tab == tab;
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .relative()
                    .h(px(34.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.text_color(text()).child(
                            div()
                                .absolute()
                                .bottom(px(-1.0))
                                .left(px(8.0))
                                .right(px(8.0))
                                .h(px(2.0))
                                .rounded(px(1.0))
                                .bg(accent_main()),
                        )
                    })
                    .when(!selected, |d| d.text_color(text_dim()).hover(|s| s.text_color(text())))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        this.menu = None;
                        this.log_generation = u64::MAX;
                        this.log_follow = true;
                        cx.notify();
                    })),
            );
        }
        bar
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings.show_developer && self.tab == Tab::Developer {
            // Nothing extra: the tab draws its own inputs.
        } else if self.tab == Tab::Developer {
            self.tab = Tab::Camera;
        }
        let frame = titlebar::Frame::of(window);
        let header = self.render_header(frame, window, cx);
        let shown = self.shown();
        let tabs_bar = self.render_tabs_bar(cx);
        let tab_content = self.render_tab(window, cx);
        let controls = div()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .bg(panel())
            .border_1()
            .border_color(border())
            .rounded(px(PANEL_RADIUS))
            .p(px(12.0))
            .flex_none()
            .child(tabs_bar)
            .child(if self.tab == Tab::Log {
                // The log scrolls by itself, so it can fill the panel.
                div().id("tab-scroll").flex().flex_col().flex_1().min_h(px(0.0)).pt(px(6.0)).child(tab_content)
            } else {
                div()
                    .id("tab-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .overflow_y_scroll()
                    .pt(px(6.0))
                    .pr(px(4.0))
                    .child(tab_content)
            });

        // The width the panels share: the content without its padding.
        let inner = content_width(window, frame) - 2.0 * PADDING;
        let body = if self.transition.is_none() && shown == 0.0 {
            div().flex().flex_1().min_h(px(0.0)).child(controls.flex_1())
        } else {
            let toolbar = self.render_preview_toolbar(cx);
            let area = self.render_preview_area(cx);
            let preview = div()
                .flex()
                .flex_col()
                .size_full()
                .gap(px(8.0))
                .bg(panel())
                .border_1()
                .border_color(border())
                .rounded(px(PANEL_RADIUS))
                .p(px(12.0))
                .child(toolbar)
                .child(area);
            if self.transition.is_none() {
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(GAP))
                    .child(controls.w(px(PANEL_WIDTH)))
                    .child(div().flex().flex_1().min_w(px(160.0)).child(preview))
            } else {
                // Switching views: the controls panel on the left grows or shrinks
                // between its two widths while the preview on its right, kept at its
                // full-view width so nothing reflows, is uncovered or covered by the
                // window's right edge and fades.
                // When the window resizes, the ends are its compact and full-view widths;
                // when it's maximised, the controls fill the screen in compact view.
                let inset = frame.horizontal_inset();
                let (compact_inner, full_inner) = match self.transition.and_then(|(_, t)| t.ends) {
                    Some((compact, full)) => (compact - inset - 2.0 * PADDING, full - inset - 2.0 * PADDING),
                    None => (inner, inner),
                };
                let preview_width = (full_inner - GAP - PANEL_WIDTH).max(160.0);
                let controls_width = compact::lerp(compact_inner, PANEL_WIDTH, shown);
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(controls.w(px(controls_width)))
                    .child(div().flex_none().w(px(GAP * shown)))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .opacity(shown)
                            .child(div().flex().flex_none().w(px(preview_width)).child(preview)),
                    )
            }
        };

        let in_use_note = (self.compact && self.camera_use.in_use)
            .then(|| note(format!("Camera in use by {}, so the preview is hidden.", self.camera_use_name())));
        let menu = self.render_menu(cx);

        let root = div()
            .id("root")
            .size_full()
            .flex()
            .flex_col()
            .bg(bg())
            .text_color(text())
            .font_family(FONT_FAMILY)
            .text_size(px(TEXT_SIZE))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(GAP))
                    .px(px(PADDING))
                    .pb(px(PADDING))
                    .when_some(in_use_note, |d, n| d.child(n))
                    .child(body),
            )
            .when_some(menu, |d, m| d.child(m))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    let had = this.drag.is_some() || this.held.is_some();
                    if this.drag.take() == Some(SliderId::MoveSpeed) {
                        this.save_settings();
                    }
                    this.release_hold();
                    if had {
                        cx.notify();
                    }
                    return;
                }
                if let Some(id) = this.drag {
                    this.drag_slider(id, event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_press(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_press(cx)));
        let title_state = self.title_state.clone();
        titlebar::client_frame(root, frame, &title_state, window)
    }
}

impl AppView {
    fn end_press(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.drag.take() {
            if id == SliderId::MoveSpeed {
                self.save_settings();
            }
            cx.notify();
        }
        if self.held.is_some() {
            self.release_hold();
            cx.notify();
        }
    }
}

/// Pan/tilt ranges larger than an hour of arc are in arc-seconds.
fn angle_scale(min: i32, max: i32) -> f64 {
    if (max as i64 - min as i64) > 3600 {
        3600.0
    } else {
        1.0
    }
}

/// Wraps tightly packed BGRA pixels for GPUI.
fn render_image(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

/// HUD-style corner brackets around a rectangle.
fn paint_brackets(window: &mut Window, rect: Bounds<Pixels>, colour: gpui::Hsla) {
    let (w, h) = (f32::from(rect.size.width), f32::from(rect.size.height));
    let len = (w * 0.25).min(h * 0.25).min(18.0);
    let t = 1.5;
    let (x0, y0) = (f32::from(rect.origin.x), f32::from(rect.origin.y));
    let (x1, y1) = (x0 + w, y0 + h);
    let bar = |x: f32, y: f32, bw: f32, bh: f32| Bounds::new(point(px(x), px(y)), size(px(bw), px(bh)));
    for b in [
        bar(x0, y0, len, t),
        bar(x0, y0, t, len),
        bar(x1 - len, y0, len, t),
        bar(x1 - t, y0, t, len),
        bar(x0, y1 - t, len, t),
        bar(x0, y1 - len, t, len),
        bar(x1 - len, y1 - t, len, t),
        bar(x1 - t, y1 - len, t, len),
    ] {
        window.paint_quad(fill(b, colour));
    }
}
