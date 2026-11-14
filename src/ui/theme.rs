//! Palette, fonts and sizes. Near-black with a faint teal tint and one neon accent
//! for everything that is on, selected or active. Green, amber and red are only for
//! status (the connection pill, log levels, warnings).

use gpui::{rgb, rgba, Hsla, Rgba};

/// The user's neon cyan, #67F7FF, at a given strength. Highlights use 85% of it so
/// they don't overpower plain text on high-contrast (OLED) screens; hover uses it in full.
pub fn accent(alpha: f32) -> Hsla {
    let mut c: Hsla = rgb(0x67F7FF).into();
    c.a = alpha;
    c
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn bg() -> Hsla {
    hex(0x06090D)
}
pub fn panel() -> Hsla {
    hex(0x0B1117)
}
pub fn frame() -> Hsla {
    hex(0x15222C)
}
pub fn frame_hover() -> Hsla {
    hex(0x1C2E3A)
}
pub fn frame_active() -> Hsla {
    hex(0x243B4A)
}
pub fn border() -> Hsla {
    hex(0x263C49)
}
pub fn text() -> Hsla {
    hex(0xEAF6F8)
}
pub fn text_dim() -> Hsla {
    hex(0x92ADB6)
}
pub fn accent_main() -> Hsla {
    accent(0.85)
}
pub fn accent_hover() -> Hsla {
    accent(1.0)
}
pub fn accent_dim() -> Hsla {
    accent(0.25)
}
/// The background of a selected button.
pub fn accent_tint() -> Hsla {
    accent(0.08)
}
pub fn good() -> Hsla {
    hex(0x4DFF9A)
}
pub fn warn() -> Hsla {
    hex(0xFFB547)
}
pub fn bad() -> Hsla {
    hex(0xFF4D6D)
}
pub fn transparent() -> Hsla {
    let c: Rgba = rgba(0x00000000);
    c.into()
}

/// Geist and Geist Mono (SIL Open Font Licence 1.1), built into the app.
pub const FONT_FAMILY: &str = "Geist";
pub const MONO_FAMILY: &str = "Geist Mono";
pub static FONTS: [&[u8]; 3] = [
    include_bytes!("../../third_party/fonts/geist/Geist-Regular.ttf"),
    include_bytes!("../../third_party/fonts/geist/Geist-SemiBold.ttf"),
    include_bytes!("../../third_party/fonts/geist/GeistMono-Regular.ttf"),
];

/// Base text size in pixels.
pub const TEXT_SIZE: f32 = 15.0;
/// Width of the label column in a control row.
pub const LABEL_WIDTH: f32 = 136.0;
/// The narrower label column beside the arrow pad.
pub const COMPACT_LABEL_WIDTH: f32 = 44.0;
/// Width of the value column (numbers are right-aligned in it).
pub const VALUE_WIDTH: f32 = 46.0;
/// Room after a number for its unit ("°/s"), so digits line up with or without one.
pub const UNIT_WIDTH: f32 = 24.0;
/// Height of buttons and other controls.
pub const CONTROL_HEIGHT: f32 = 30.0;
/// The controls panel in full view.
pub const PANEL_WIDTH: f32 = 470.0;
/// Below this content width the header uses its short form (see `render_header`).
pub const HEADER_WIDE: f32 = 760.0;
/// The window's content width in compact view: the panel and some padding.
pub const COMPACT_WIDTH: f32 = 520.0;
