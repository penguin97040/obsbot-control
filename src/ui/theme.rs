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
    accent(0.10)
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

// ---- Type scale, tuned for Geist (its lowercase is 53% of the size, so it reads
// larger than most fonts at the same size).

/// Body text: labels, buttons, values.
pub const TEXT_SIZE: f32 = 13.0;
/// Notes, tooltips, the status pill and other secondary text.
pub const SMALL_SIZE: f32 = 12.0;
/// Section headings (semibold, accent).
pub const HEADING_SIZE: f32 = 12.0;
/// The wordmark in the title bar.
pub const WORDMARK_SIZE: f32 = 14.0;

// ---- Sizes and corners. Corners step down from panels to controls to the chips
// inside them, so nested shapes look concentric.

/// Height of buttons and other controls.
pub const CONTROL_HEIGHT: f32 = 28.0;
pub const PANEL_RADIUS: f32 = 10.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const CHIP_RADIUS: f32 = 4.0;
/// Width of the label column in a control row.
pub const LABEL_WIDTH: f32 = 112.0;
/// The narrower label column beside the arrow pad.
pub const COMPACT_LABEL_WIDTH: f32 = 40.0;
/// Width of the value column (numbers are right-aligned in it).
pub const VALUE_WIDTH: f32 = 40.0;
/// Room after a number for its unit ("°/s"), so digits line up with or without one.
pub const UNIT_WIDTH: f32 = 22.0;
/// The controls panel in full view.
pub const PANEL_WIDTH: f32 = 440.0;
/// Below this content width the header uses its short form (see `render_header`).
pub const HEADER_WIDE: f32 = 700.0;
/// The window's content width in compact view: the panel and some padding.
pub const COMPACT_WIDTH: f32 = 480.0;
