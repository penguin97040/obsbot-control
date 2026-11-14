//! The contents of each tab.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, ClipboardItem, Context, KeyDownEvent, MouseButton, SharedString, Window};

use super::theme::*;
use super::widgets::*;
use super::{AppView, DevField, Menu, SliderId, Tab};
use crate::camera::worker::VariantSetting;
use crate::camera::{UvcCtl, POWER_LINE_50HZ, POWER_LINE_60HZ, POWER_LINE_DISABLED};
use crate::protocol::{self, AiMode, Fov, Receiver, Variant};
use crate::settings::{Preset, Settings, MOVE_GIMBAL_SPEED};
use crate::{log_info, log_warn, logx, platform};

const DESK_TIP: &str = "Desk mode may not work on every firmware version.";

fn anti_flicker_name(v: i32) -> &'static str {
    match v {
        POWER_LINE_DISABLED => "Off",
        POWER_LINE_50HZ => "50 Hz",
        POWER_LINE_60HZ => "60 Hz",
        _ => "Auto",
    }
}

impl AppView {
    pub(super) fn render_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            Tab::Control => self.control_tab(cx).into_any_element(),
            Tab::Image => self.image_tab(cx).into_any_element(),
            Tab::Camera => self.camera_tab(cx).into_any_element(),
            Tab::Log => self.log_tab(cx).into_any_element(),
            Tab::Developer => self.developer_tab(window, cx).into_any_element(),
        }
    }

    // ---- Control tab ---------------------------------------------------------------------

    fn control_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap(px(8.0)).child(self.ai_tracking(cx)).child(self.gimbal(cx))
    }

    fn ai_tracking(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let enabled = s.connected && s.xu;
        let current = if s.status.valid || s.status.ai_mode.is_some() { s.status.ai_mode } else { None };
        let mut col = div().flex().flex_col().gap(px(8.0)).child(section_title("AI tracking"));
        col = col.child(
            toggle_button(
                "ai-off",
                "Tracking off",
                current == Some(AiMode::Off),
                enabled,
                Some(Box::new(cx.listener(|this, _, _, _| this.worker.set_ai_mode(AiMode::Off)))),
            )
            .w_full(),
        );
        let modes = [
            AiMode::Normal,
            AiMode::UpperBody,
            AiMode::CloseUp,
            AiMode::Headless,
            AiMode::LowerBody,
            AiMode::Group,
            AiMode::Hand,
            AiMode::Whiteboard,
            AiMode::Desk,
        ];
        for chunk in modes.chunks(3) {
            let mut row = div().flex().gap(px(8.0));
            for &m in chunk {
                let label = if m == AiMode::Normal { "Normal" } else { m.name() };
                let mut b = toggle_button(
                    SharedString::from(format!("ai-{m:?}")),
                    label,
                    current == Some(m),
                    enabled,
                    Some(Box::new(cx.listener(move |this, _, _, _| this.worker.set_ai_mode(m)))),
                )
                .flex_1()
                .min_w(px(0.0));
                if m == AiMode::Desk {
                    b = b.tooltip(tip(DESK_TIP));
                }
                row = row.child(b);
            }
            col = col.child(row);
        }
        col = col.child(row(
            "speed-label",
            "Speed",
            None,
            segmented(
                "track-speed",
                &["Standard", "Sport"],
                &["Smooth, steady framing.", "Follows fast movement more closely."],
                Some(s.status.sport_tracking as usize),
                enabled,
                cx.listener(|this, i: &usize, _, _| this.worker.set_sport_tracking(*i == 1)),
            ),
        ));
        if s.status.valid && s.status.ai_mode.is_none() {
            col = col.child(note(format!("Camera reports an unknown mode ({}, {}).", s.status.ai_m, s.status.ai_n)));
        }
        if s.connected && !s.xu {
            col = col.child(note("AI controls need the OBSBOT extension unit (see Log)."));
        }
        col
    }

    fn gimbal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let connected = s.connected;
        let size = 46.0;
        let gap = 4.0;
        let held = self.held;
        let arrow = |id: &'static str, a: Arrow, dx: i32, dy: i32, cx: &mut Context<Self>| {
            arrow_button(
                id,
                a,
                held == Some((dx, dy)),
                size,
                connected,
                cx.listener(move |this, _, _, cx| {
                    this.start_hold(dx, dy);
                    cx.notify();
                }),
            )
        };
        let centre = div()
            .id("centre")
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .border_1()
            .bg(frame())
            .border_color(border())
            .child(
                div()
                    .size(px(size * 0.4))
                    .rounded(px(size * 0.2))
                    .border_2()
                    .border_color(text())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(size * 0.13)).rounded(px(size * 0.07)).bg(text())),
            )
            .tooltip(tip("Centre the camera"))
            .when(!(connected && s.xu), |d| d.opacity(0.55))
            .when(connected && s.xu, |d| {
                d.cursor_pointer()
                    .hover(|st| st.bg(frame_hover()))
                    .on_click(cx.listener(|this, _, _, _| this.worker.recentre()))
            });
        let spacer = || div().size(px(size)).flex_none();
        let pad = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(gap))
            .child(div().flex().gap(px(gap)).child(spacer()).child(arrow("up", Arrow::Up, 0, 1, cx)).child(spacer()))
            .child(div().flex().gap(px(gap)).child(arrow("left", Arrow::Left, -1, 0, cx)).child(centre).child(arrow(
                "right",
                Arrow::Right,
                1,
                0,
                cx,
            )))
            .child(
                div().flex().gap(px(gap)).child(spacer()).child(arrow("down", Arrow::Down, 0, -1, cx)).child(spacer()),
            );

        let sliders = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .justify_center()
            .gap(px(4.0))
            .child(self.angle_row(UvcCtl::Pan, "Pan", cx))
            .child(self.angle_row(UvcCtl::Tilt, "Tilt", cx))
            .child(self.uvc_row(UvcCtl::Zoom, "Zoom", COMPACT_LABEL_WIDTH, false, true, cx));

        let smooth = self.settings.move_method == MOVE_GIMBAL_SPEED;
        let speed_row = div()
            .flex()
            .items_center()
            .min_h(px(CONTROL_HEIGHT))
            .child(label_cell("move-speed-label", "Speed", LABEL_WIDTH, None))
            .child(
                div()
                    .id("move-speed")
                    .flex()
                    .flex_1()
                    .items_center()
                    .child(self.slider(SliderId::MoveSpeed, cx))
                    .when(!smooth, |d| d.tooltip(tip("Only used by Smooth movement."))),
            )
            .child(
                value_text(format!("{:.0}", self.settings.move_speed), "\u{00B0}/s", true)
                    .when(!smooth, |d| d.opacity(0.55)),
            );

        let invert = div()
            .flex()
            .items_center()
            .gap(px(24.0))
            .child(
                toggle_switch(
                    "invert-h",
                    Some("Horizontal"),
                    self.settings.invert_horizontal,
                    true,
                    cx.listener(|this, on: &bool, _, cx| {
                        this.settings.invert_horizontal = *on;
                        this.settings.save();
                        cx.notify();
                    }),
                )
                .tooltip(tip("Swap the left and right arrows.")),
            )
            .child(
                toggle_switch(
                    "invert-v",
                    Some("Vertical"),
                    self.settings.invert_vertical,
                    true,
                    cx.listener(|this, on: &bool, _, cx| {
                        this.settings.invert_vertical = *on;
                        this.settings.save();
                        cx.notify();
                    }),
                )
                .tooltip(tip("Swap the up and down arrows.")),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(section_title("Gimbal and zoom"))
            .child(div().flex().items_center().gap(px(16.0)).child(pad).child(sliders))
            .child(row(
                "method-label",
                "Movement",
                None,
                segmented(
                    "method",
                    &["Smooth (vendor)", "Steps (UVC)"],
                    &[
                        "Moves at a steady speed using OBSBOT's own command.",
                        "Moves in small standard webcam steps. Try this if Smooth doesn't move your camera.",
                    ],
                    Some(self.settings.move_method.clamp(0, 1) as usize),
                    true,
                    cx.listener(|this, i: &usize, _, cx| {
                        this.settings.move_method = *i as i32;
                        this.settings.save();
                        cx.notify();
                    }),
                ),
            ))
            .child(speed_row)
            .child(row("invert-label", "Invert", None, invert))
            .child(self.presets(cx))
    }

    /// One row of three presets: click an empty one to save the current position, a
    /// saved one to go back to it; right-click either to update or clear it.
    fn presets(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let usable = s.connected && s.uvc(UvcCtl::Pan).supported && s.uvc(UvcCtl::Tilt).supported;
        let mut buttons = div().flex().flex_1().gap(px(8.0));
        for (i, p) in self.settings.presets.iter().enumerate() {
            let saved = p.saved;
            let label = if saved { format!("Preset {}", i + 1) } else { format!("+ Save {}", i + 1) };
            let preset = *p;
            let mut b = div()
                .id(("preset", i))
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .h(px(CONTROL_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.0))
                .border_1()
                .bg(frame())
                .border_color(border())
                .text_color(if saved { text() } else { text_dim() })
                .whitespace_nowrap()
                .child(label)
                .when(saved, |d| {
                    // A small accent tick on the left edge marks a saved preset.
                    d.child(
                        div()
                            .absolute()
                            .left(px(3.0))
                            .top(px(8.0))
                            .bottom(px(8.0))
                            .w(px(3.0))
                            .rounded(px(2.0))
                            .bg(accent_main()),
                    )
                })
                .tooltip(tip(if saved {
                    "Go to this position. Right-click to update or clear."
                } else {
                    "Save where the camera is pointing now."
                }));
            if usable {
                b = b
                    .cursor_pointer()
                    .hover(|st| st.bg(frame_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if preset.saved {
                            this.worker.goto_position(preset.pan, preset.tilt, preset.zoom);
                        } else {
                            this.save_preset(i);
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.menu = Some(Menu::Preset(i, event.position));
                            cx.notify();
                        }),
                    );
            } else {
                b = b.opacity(0.55);
            }
            buttons = buttons.child(b);
        }
        let mut col = div().flex().flex_col().gap(px(6.0)).child(row(
            "presets-label",
            "Presets",
            Some(
                "Click an empty preset to save where the camera is pointing. Click a saved one to go back there. Right-click a preset to update or clear it.",
            ),
            buttons,
        ));
        if s.connected && !usable {
            col = col.child(note("Presets need UVC pan/tilt support, which this camera doesn't report."));
        }
        col
    }

    fn save_preset(&mut self, i: usize) {
        let s = &self.state;
        let p = Preset {
            saved: true,
            pan: s.uvc(UvcCtl::Pan).value,
            tilt: s.uvc(UvcCtl::Tilt).value,
            zoom: s.uvc(UvcCtl::Zoom).value,
        };
        self.settings.presets[i] = p;
        self.settings.save();
        log_info!("Saved preset {} (pan {}, tilt {}, zoom {}).", i + 1, p.pan, p.tilt, p.zoom);
    }

    pub(super) fn preset_menu_items(&mut self, i: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let saved = self.settings.presets[i].saved;
        vec![
            menu_item(
                "preset-save",
                if saved { "Update to the current position" } else { "Save the current position" },
                false,
                true,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.save_preset(i);
                    this.menu = None;
                    cx.notify();
                })),
            )
            .into_any_element(),
            menu_item(
                "preset-clear",
                "Clear",
                false,
                saved,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.settings.presets[i] = Preset::default();
                    this.settings.save();
                    this.menu = None;
                    cx.notify();
                })),
            )
            .into_any_element(),
        ]
    }

    // ---- Image tab -------------------------------------------------------------------------

    fn image_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.state.connected;
        let rows = |view: &AppView, list: &[(UvcCtl, &str)], cx: &mut Context<Self>| {
            list.iter().map(|(c, label)| view.uvc_row(*c, label, LABEL_WIDTH, true, false, cx)).collect::<Vec<_>>()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(section_title("Picture"))
            .children(rows(
                self,
                &[
                    (UvcCtl::Brightness, "Brightness"),
                    (UvcCtl::Contrast, "Contrast"),
                    (UvcCtl::Saturation, "Saturation"),
                    (UvcCtl::Sharpness, "Sharpness"),
                    (UvcCtl::Hue, "Hue"),
                    (UvcCtl::Gamma, "Gamma"),
                ],
                cx,
            ))
            .child(section_title("Colour and light"))
            .children(rows(
                self,
                &[
                    (UvcCtl::WhiteBalance, "White balance"),
                    (UvcCtl::Exposure, "Exposure"),
                    (UvcCtl::Gain, "Gain"),
                    (UvcCtl::BacklightComp, "Backlight"),
                ],
                cx,
            ))
            .child(section_title("Focus"))
            .children(rows(self, &[(UvcCtl::Focus, "Focus")], cx))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .pt(px(8.0))
                    .child(
                        button(
                            "reset",
                            "Reset to defaults",
                            connected,
                            Some(Box::new(cx.listener(|this, _, _, _| this.worker.reset_image_defaults()))),
                        )
                        .flex_1()
                        .tooltip(tip("Put every picture setting back to the camera's own defaults.")),
                    )
                    .child(
                        button(
                            "refresh",
                            "Refresh",
                            connected,
                            Some(Box::new(cx.listener(|this, _, _, _| this.worker.refresh_all()))),
                        )
                        .flex_1()
                        .tooltip(tip("Read every setting from the camera again.")),
                    ),
            )
    }

    // ---- Camera tab ------------------------------------------------------------------------

    fn camera_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let xu = s.connected && s.xu;
        let current_fov = if s.status.valid { Fov::from_status(s.status.fov_raw, s.variant) } else { None };
        let power = *s.uvc(UvcCtl::PowerLine);
        let power_values = [POWER_LINE_DISABLED, POWER_LINE_50HZ, POWER_LINE_60HZ];
        let power_selected = power_values.iter().position(|&v| v == self.settings.anti_flicker);
        let hdr = s.status.hdr;

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(section_title("Picture mode"))
            .child(row(
                "hdr-label",
                "HDR",
                Some("High dynamic range: helps when there's a bright window behind you. May lower the frame rate."),
                toggle_switch("hdr", None, hdr, xu, cx.listener(|this, on: &bool, _, _| this.worker.set_hdr(*on))),
            ))
            .child(row(
                "fov-label",
                "Field of view",
                None,
                segmented(
                    "fov",
                    &["Wide", "Medium", "Narrow"],
                    &[],
                    current_fov.map(|f| f as usize),
                    xu,
                    cx.listener(|this, i: &usize, _, _| this.worker.set_fov(Fov::ALL[*i])),
                ),
            ))
            .child(section_title("Anti-flicker"))
            .child(row(
                "power-label",
                "Mains power",
                Some("Matches the camera to your mains power to stop lights flickering on video. New Zealand and Australia use 50 Hz. Applied each time the camera connects."),
                segmented(
                    "power",
                    &["Off", "50 Hz", "60 Hz"],
                    &[],
                    power_selected,
                    s.connected && power.supported,
                    cx.listener(move |this, i: &usize, _, cx| {
                        let v = power_values[*i];
                        this.settings.anti_flicker = v;
                        this.worker.set_anti_flicker_on_connect(v);
                        this.worker.set_uvc(UvcCtl::PowerLine, v, false);
                        this.settings.save();
                        cx.notify();
                    }),
                ),
            ));
        if s.connected && power.supported {
            col = col.child(
                div()
                    .pl(px(LABEL_WIDTH))
                    .child(note(format!("Camera currently reports: {}", anti_flicker_name(power.value)))),
            );
        }

        let switch_row = |id: &'static str,
                          label: &str,
                          help: &'static str,
                          on: bool,
                          apply: fn(&mut Settings, bool),
                          reset_use: bool,
                          cx: &mut Context<Self>| {
            row(
                SharedString::from(format!("{id}-label")),
                label,
                Some(help),
                toggle_switch(
                    id,
                    None,
                    on,
                    true,
                    cx.listener(move |this, on: &bool, _, cx| {
                        apply(&mut this.settings, *on);
                        if reset_use {
                            this.policy = Default::default();
                            this.camera_use = Default::default();
                            this.last_use_check = None; // check straight away if it was turned on
                        }
                        this.settings.save();
                        cx.notify();
                    }),
                ),
            )
        };
        col = col
            .child(section_title("Advanced"))
            .child(switch_row(
                "auto-compact",
                "Auto compact",
                "Switches to compact view (no preview, narrower window) while another app such as Zoom, Discord or Teams is using the camera, and back again when it stops.",
                self.settings.auto_compact,
                |s, on| s.auto_compact = on,
                true,
                cx,
            ))
            .child(switch_row(
                "pause-bg",
                "Pause preview",
                "Stops the preview while you're using another window, so apps like Discord or Zoom can turn on the camera. It starts again when you come back.",
                self.settings.pause_in_background,
                |s, on| s.pause_in_background = on,
                false,
                cx,
            ))
            .child(switch_row(
                "developer",
                "Developer tab",
                "Adds a Developer tab for testing raw camera commands. You won't need it for everyday use.",
                self.settings.show_developer,
                |s, on| s.show_developer = on,
                false,
                cx,
            ));

        col.child(section_title("About"))
            .child(div().child(format!("obsbot-control {}", env!("CARGO_PKG_VERSION"))))
            .child(div().child("Free and open-source software under the MIT Licence. Unofficial: not made by, affiliated with or endorsed by OBSBOT."))
            .child(div().child("Private by design: works entirely offline, with no accounts, telemetry or internet access."))
            .child(note("Uses the Chakra Petch font (SIL Open Font Licence 1.1) and the GPUI user interface library (Apache Licence 2.0)."))
            .child(div().pt(px(4.0)).child(button(
                "open-folder",
                "Open settings folder",
                Settings::directory().is_some(),
                Some(Box::new(cx.listener(|_, _, _, cx| {
                    if let Some(dir) = Settings::directory() {
                        cx.open_with_system(&dir);
                    }
                }))),
            )))
    }

    // ---- Log tab -----------------------------------------------------------------------------

    pub(super) fn diagnostics(&self) -> String {
        let s = &self.state;
        let mut d = format!(
            "obsbot-control {} diagnostics, {}\n{}\n",
            env!("CARGO_PKG_VERSION"),
            chrono::Local::now().format("%-d/%m/%Y %H:%M"),
            platform::os_version()
        );
        if !s.connected {
            d.push_str("Camera: not connected\n");
        } else {
            d.push_str(&format!(
                "Camera: {} ({}), VID {:04X} PID {:04X}\nPath: {}\n",
                s.device.name,
                s.device.model.name(),
                s.device.vid,
                s.device.pid,
                s.device.path
            ));
            d.push_str(&format!(
                "Extension unit: {} ({} {}), protocol variant {}\n",
                if s.xu { "found" } else { "NOT found" },
                if cfg!(windows) { "node" } else { "unit" },
                s.xu_location,
                s.variant.label()
            ));
            for c in UvcCtl::ALL {
                let u = s.uvc(c);
                if !u.supported {
                    d.push_str(&format!("  {:<13} unsupported\n", c.diag_name()));
                } else {
                    d.push_str(&format!(
                        "  {:<13} {}..{} step {} def {} value {}{}{}\n",
                        c.diag_name(),
                        u.min,
                        u.max,
                        u.step,
                        u.def,
                        u.value,
                        if u.can_auto { " canAuto" } else { "" },
                        if u.is_auto { " AUTO" } else { "" }
                    ));
                }
            }
            d.push_str(&format!("Status block: {}\n", protocol::to_hex(&s.raw_status)));
        }
        let preview = if self.preview.running() { self.preview.format_label() } else { "not running".into() };
        d.push_str(&format!("Preview: {preview}\n"));
        let error = self.preview.error();
        if !error.is_empty() {
            d.push_str(&format!("Preview error: {error}\n"));
        }
        d.push_str("\n--- Log ---\n");
        d.push_str(&logx::all_as_text());
        d
    }

    fn log_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .log_entries
            .iter()
            .map(|e| {
                let colour = match e.level {
                    logx::Level::Error => bad(),
                    logx::Level::Warn => warn(),
                    logx::Level::Info => text(),
                };
                div()
                    .flex()
                    .gap(px(8.0))
                    .py(px(1.0))
                    .text_size(px(13.0))
                    .child(div().flex_none().text_color(text_dim()).child(e.time.clone()))
                    .child(div().flex_1().min_w(px(0.0)).text_color(colour).child(e.text.clone()))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        button(
                            "copy-diagnostics",
                            "Copy diagnostics",
                            true,
                            Some(Box::new(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.diagnostics()));
                                log_info!("Diagnostics copied to the clipboard.");
                            }))),
                        )
                        .flex_1()
                        .tooltip(tip("Copies camera details and this log so you can paste them into a bug report.")),
                    )
                    .child(
                        button("clear-log", "Clear log", true, Some(Box::new(cx.listener(|_, _, _, _| logx::clear()))))
                            .flex_1(),
                    ),
            )
            .child(
                div()
                    .id("log-box")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.log_scroll)
                    .p(px(8.0))
                    .bg(bg())
                    .border_1()
                    .border_color(border())
                    .rounded(px(3.0))
                    .on_scroll_wheel(cx.listener(|this, _, _, _| {
                        // Follow new entries again once scrolled back to the bottom.
                        let offset = this.log_scroll.offset().y;
                        let max = this.log_scroll.max_offset().height;
                        this.log_follow = -offset >= max - px(4.0);
                    }))
                    .children(rows),
            )
    }

    // ---- Developer tab -------------------------------------------------------------------------

    fn text_field(
        &self,
        field: DevField,
        placeholder: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let index = field as usize;
        let handle = self.dev_focus[index].clone();
        let focused = handle.is_focused(window);
        let value = match field {
            DevField::Hex => self.dev_hex.clone(),
            DevField::Command => self.dev_command.clone(),
            DevField::Payload => self.dev_payload.clone(),
        };
        let shown = if value.is_empty() && !focused {
            div().text_color(text_dim()).child(placeholder.to_string())
        } else {
            div().child(format!("{value}{}", if focused { "\u{2502}" } else { "" }))
        };
        let click_handle = handle.clone();
        div()
            .id(("dev-field", index))
            .track_focus(&handle)
            .flex_1()
            .min_w(px(0.0))
            .h(px(CONTROL_HEIGHT))
            .px(px(8.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .rounded(px(3.0))
            .border_1()
            .bg(frame())
            .border_color(if focused { accent_main() } else { border() })
            .cursor_text()
            .child(shown)
            .on_click(cx.listener(move |_, _, window, cx| {
                window.focus(&click_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let text = match field {
                    DevField::Hex => &mut this.dev_hex,
                    DevField::Command => &mut this.dev_command,
                    DevField::Payload => &mut this.dev_payload,
                };
                let ks = &event.keystroke;
                if ks.key == "backspace" {
                    text.pop();
                } else if ks.key == "escape" || ks.key == "enter" {
                } else if !ks.modifiers.control && !ks.modifiers.platform && !ks.modifiers.alt {
                    if let Some(ch) = ks.key_char.as_deref() {
                        // Only what hex input can use, so the field can't fill with junk.
                        for c in ch.chars().filter(|c| c.is_ascii_hexdigit() || " ,:-xX".contains(*c)) {
                            if text.len() < 240 {
                                text.push(c);
                            }
                        }
                    }
                }
                cx.notify();
            }))
    }

    fn developer_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let usable = s.connected && s.xu;
        let variant_now = if s.variant == Variant::Lite { "Tiny 2 Lite" } else { "Tiny 2" };
        let status = if s.status.valid {
            let st = s.status;
            div()
                .flex()
                .flex_col()
                .child(format!(
                    "Asleep {}, device status {}, HDR {}, FOV byte {}, zoom {}%",
                    if st.asleep { "yes" } else { "no" },
                    st.device_status,
                    if st.hdr { "on" } else { "off" },
                    st.fov_raw,
                    st.zoom_percent
                ))
                .child(format!(
                    "AI bytes ({}, {}) = {}, tracking {}",
                    st.ai_m,
                    st.ai_n,
                    st.ai_mode.map(|m| m.name()).unwrap_or("unknown"),
                    if st.sport_tracking { "sport" } else { "standard" }
                ))
        } else {
            div().child(note("No status yet."))
        };
        let selector = self.dev_selector;
        let stepper = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(button(
                "sel-down",
                "\u{2212}",
                true,
                Some(Box::new(cx.listener(|this, _, _, cx| {
                    this.dev_selector = this.dev_selector.saturating_sub(1).max(1);
                    cx.notify();
                }))),
            ))
            .child(div().w(px(32.0)).flex().justify_center().child(selector.to_string()))
            .child(button(
                "sel-up",
                "+",
                true,
                Some(Box::new(cx.listener(|this, _, _, cx| {
                    this.dev_selector = (this.dev_selector + 1).min(31);
                    cx.notify();
                }))),
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().text_color(warn()).child(
                "For testing protocol commands. Unknown commands can confuse the camera; unplug it and plug it back in to recover.",
            ))
            .child(section_title("Protocol variant"))
            .child(segmented(
                "variant",
                &["Automatic (by model)", "Tiny 2 codes", "Tiny 2 Lite codes"],
                &[],
                Some(self.settings.variant.clamp(0, 2) as usize),
                true,
                cx.listener(|this, i: &usize, _, cx| {
                    this.settings.variant = *i as i32;
                    this.worker.set_variant_setting(VariantSetting::from_index(*i as i32));
                    this.settings.save();
                    cx.notify();
                }),
            ))
            .child(note(format!("In use: {variant_now} codes. Affects field of view and sleep commands.")))
            .child(section_title("Status block (selector 6)"))
            .child(status)
            .child(note(protocol::to_hex(&s.raw_status)))
            .child(section_title("Raw XU transfer"))
            .child(row("selector-label", "Selector", None, stepper))
            .child(div().flex().child(self.text_field(DevField::Hex, "Bytes in hex, e.g. 16 02 02 01", window, cx)))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button("dev-set", "SET", usable, Some(Box::new(cx.listener(|this, _, _, _| {
                        match protocol::parse_hex(&this.dev_hex) {
                            Some(bytes) => this.worker.dev_set_raw(this.dev_selector, bytes),
                            None => log_warn!("[dev] Couldn't read those hex bytes."),
                        }
                    })))))
                    .child(button("dev-get", "GET", usable, Some(Box::new(cx.listener(|this, _, _, _| {
                        this.worker.dev_get_raw(this.dev_selector);
                    }))))),
            )
            .child(section_title("Framed command (selector 2)"))
            .child(row(
                "receiver-label",
                "Receiver",
                None,
                segmented(
                    "receiver",
                    &["Camera (0x02)", "Gimbal (0x03)", "AI (0x04)"],
                    &[],
                    Some(self.dev_receiver),
                    true,
                    cx.listener(|this, i: &usize, _, cx| {
                        this.dev_receiver = *i;
                        cx.notify();
                    }),
                ),
            ))
            .child(row("command-label", "Command", None, self.text_field(DevField::Command, "e.g. A0C2", window, cx)))
            .child(div().flex().child(self.text_field(DevField::Payload, "Payload hex (empty = GET)", window, cx)))
            .child(div().flex().child(button("dev-framed", "Send framed", usable, Some(Box::new(cx.listener(|this, _, _, _| {
                let command = u16::from_str_radix(this.dev_command.trim().trim_start_matches("0x").trim_start_matches("0X"), 16);
                match (protocol::parse_hex(&this.dev_payload), command) {
                    (Some(payload), Ok(command)) => {
                        let receiver = match this.dev_receiver {
                            0 => Receiver::Camera,
                            1 => Receiver::Gimbal,
                            _ => Receiver::Ai,
                        };
                        this.worker.dev_framed(receiver, command, payload);
                    }
                    _ => log_warn!("[dev] Check the command and payload hex."),
                }
            }))))))
            .child(section_title("Last result"))
            .child(
                div()
                    .min_h(px(90.0))
                    .p(px(8.0))
                    .bg(bg())
                    .border_1()
                    .border_color(border())
                    .rounded(px(3.0))
                    .text_size(px(13.0))
                    .children(s.dev_output.lines().map(|l| div().child(l.to_string())).collect::<Vec<_>>()),
            )
    }
}
