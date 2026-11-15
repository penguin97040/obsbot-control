//! The small widget kit every control row uses, so things line up across sections:
//!   label | control (stretches) | value (right-aligned) | trailing (Auto switch)
//! Each builder takes ready-made event handlers (see `Context::listener`).
//!
//! Sizes and corners come from `theme`. Odd and even sizes are paired on purpose
//! (a 6 px dot in a 20 px box, a 10 px knob in a 14 px track) so centred shapes land
//! on whole pixels instead of looking a hair off.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, linear_color_stop, linear_gradient, point, px, AnyView, App, Bounds, BoxShadow, ClickEvent, Div,
    ElementId, Font, FontFeatures, FontStyle, FontWeight, Hsla, MouseButton, MouseDownEvent, PathBuilder, Pixels,
    SharedString, Stateful, Window,
};

use super::theme::{
    self, CHIP_RADIUS, CONTROL_HEIGHT, CONTROL_RADIUS, HEADING_SIZE, LABEL_WIDTH, SMALL_SIZE, TEXT_SIZE, UNIT_WIDTH,
    VALUE_WIDTH,
};

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// How strongly disabled controls fade.
const DISABLED: f32 = 0.45;

/// A soft neon halo for selected and active controls.
pub fn glow(strength: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme::accent(0.16 * strength),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(10.0),
        spread_radius: px(0.0),
    }]
}

/// Geist with tabular (equal-width) figures, so changing numbers don't shuffle sideways.
pub fn tabular_font() -> Font {
    Font {
        family: theme::FONT_FAMILY.into(),
        features: FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

struct TooltipView(SharedString);

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(320.0))
            .px(px(10.0))
            .py(px(7.0))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .rounded(px(CONTROL_RADIUS))
            .shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.5),
                offset: point(px(0.0), px(4.0)),
                blur_radius: px(12.0),
                spread_radius: px(0.0),
            }])
            .text_color(theme::text())
            .text_size(px(SMALL_SIZE))
            .line_height(px(17.0))
            .font_family(theme::FONT_FAMILY)
            .child(self.0.clone())
    }
}

/// A tooltip builder for `.tooltip(...)`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| TooltipView(text.clone())).into()
}

/// A section heading: the name in the accent colour, then a hairline that fades out
/// across the rest of the row.
pub fn section_title(text: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .pt(px(14.0))
        .pb(px(2.0))
        .child(
            div()
                .flex_none()
                .text_color(theme::accent_main())
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(HEADING_SIZE))
                .child(text.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(linear_gradient(
            90.0,
            linear_color_stop(theme::accent(0.22), 0.0),
            linear_color_stop(theme::accent(0.0), 1.0),
        )))
}

/// Dim, smaller explanatory text.
pub fn note(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::text_dim()).text_size(px(SMALL_SIZE)).line_height(px(17.0)).child(text.into())
}

/// A small circled question mark that shows help on hover.
fn help_mark() -> Div {
    div()
        .flex_none()
        .size(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .border_1()
        .border_color(theme::border())
        .text_color(theme::text_dim())
        .text_size(px(10.0))
        .line_height(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child("?")
}

/// The label column of a row (with an optional help mark and tooltip).
pub fn label_cell(id: impl Into<ElementId>, label: &str, width: f32, help: Option<&'static str>) -> Stateful<Div> {
    let cell = div()
        .id(id)
        .flex_none()
        .w(px(width))
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_color(theme::text_dim())
        .child(div().whitespace_nowrap().child(label.to_string()));
    match help {
        Some(text) => cell.child(help_mark()).tooltip(tip(text)),
        None => cell,
    }
}

/// A control row: the label, then the control filling the rest of the line.
pub fn row(id: impl Into<ElementId>, label: &str, help: Option<&'static str>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(px(CONTROL_HEIGHT))
        .child(label_cell(id, label, LABEL_WIDTH, help))
        .child(div().flex_1().min_w(px(0.0)).flex().items_center().gap(px(8.0)).child(control))
}

/// A number right-aligned in the value column, then its unit in the unit slot.
pub fn value_text(number: String, unit: &str, unit_slot: bool) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .font(tabular_font())
        .child(div().w(px(VALUE_WIDTH)).flex().justify_end().child(number))
        .when(unit_slot, |d| d.child(div().w(px(UNIT_WIDTH)).text_color(theme::text_dim()).child(unit.to_string())))
}

fn base_button(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(CONTROL_HEIGHT))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .whitespace_nowrap()
        .when(!enabled, |d| d.opacity(DISABLED))
}

/// A plain button.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: Option<ClickHandler>,
) -> Stateful<Div> {
    let b = base_button(id, enabled)
        .bg(theme::frame())
        .border_color(theme::border())
        .text_color(theme::text())
        .child(label.into());
    match on_click {
        Some(handler) if enabled => b
            .cursor_pointer()
            .hover(|s| s.bg(theme::frame_hover()))
            .active(|s| s.bg(theme::frame_active()))
            .on_click(handler),
        _ => b,
    }
}

/// A button that shows whether it is selected (a neon outline and tint when it is).
pub fn toggle_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
    on_click: Option<ClickHandler>,
) -> Stateful<Div> {
    let b = base_button(id, enabled).child(label.into());
    let b = if selected {
        b.bg(theme::accent_tint()).border_color(theme::accent_main()).text_color(theme::accent_main()).shadow(glow(1.0))
    } else {
        b.bg(theme::frame()).border_color(theme::border()).text_color(theme::text())
    };
    match on_click {
        Some(handler) if enabled => b
            .cursor_pointer()
            .hover(move |s| if selected { s.text_color(theme::accent_hover()) } else { s.bg(theme::frame_hover()) })
            .on_click(handler),
        _ => b,
    }
}

/// A choice of options in a rounded track, the selected one shown as a raised chip
/// inside it (or none, when `selected` is None). `on_select` receives the index clicked.
pub fn segmented(
    id: &str,
    options: &[&str],
    tips: &[&'static str],
    selected: Option<usize>,
    enabled: bool,
    on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = std::rc::Rc::new(on_select);
    // 28 px track: 1 px border and 2 px padding leave a 22 px chip with 4 px corners,
    // concentric with the track's 6 px ones.
    let mut track = div()
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .h(px(CONTROL_HEIGHT))
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg());
    for (i, option) in options.iter().enumerate() {
        let is_selected = selected == Some(i);
        let handler = on_select.clone();
        let mut cell = div()
            .id(SharedString::from(format!("{id}-{i}")))
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(6.0))
            .rounded(px(CHIP_RADIUS))
            .border_1()
            .whitespace_nowrap()
            .overflow_hidden()
            .child(option.to_string());
        cell = if is_selected {
            cell.bg(theme::accent(0.12)).border_color(theme::accent(0.7)).text_color(theme::accent_main())
        } else {
            cell.border_color(theme::transparent()).text_color(theme::text_dim())
        };
        if enabled {
            cell = cell
                .cursor_pointer()
                .hover(move |s| {
                    if is_selected {
                        s.text_color(theme::accent_hover())
                    } else {
                        s.bg(theme::frame()).text_color(theme::text())
                    }
                })
                .on_click(move |_, window, cx| handler(&i, window, cx));
        }
        if let Some(text) = tips.get(i) {
            cell = cell.tooltip(tip(*text));
        }
        track = track.child(cell);
    }
    track.when(!enabled, |t| t.opacity(DISABLED))
}

/// An on/off switch with an optional label after it.
pub fn toggle_switch(
    id: impl Into<ElementId>,
    label: Option<&str>,
    on: bool,
    enabled: bool,
    on_toggle: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    // 16 x 28 track with a 1 px border leaves 14 x 26 inside; a 10 px knob sits 2 px in.
    let (height, width, knob) = (16.0, 28.0, 10.0);
    let track = div()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .rounded(px(height / 2.0))
        .border_1()
        .relative()
        .when(on, |d| d.bg(theme::accent(0.22)).border_color(theme::accent_main()).shadow(glow(0.8)))
        .when(!on, |d| d.bg(theme::frame()).border_color(theme::border()))
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(if on { width - knob - 4.0 } else { 2.0 }))
                .size(px(knob))
                .rounded(px(knob / 2.0))
                .bg(if on { theme::accent_hover() } else { theme::text_dim() }),
        );
    let mut el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .h(px(CONTROL_HEIGHT))
        .child(track)
        .when_some(label, |d, text| d.child(text.to_string()))
        .when(!enabled, |d| d.opacity(DISABLED));
    if enabled {
        el = el.cursor_pointer().on_click(move |_, window, cx| on_toggle(&!on, window, cx));
    }
    el
}

/// The closed state of a dropdown: the current choice and a thin chevron.
pub fn dropdown_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    open: bool,
    enabled: bool,
) -> Stateful<Div> {
    base_button(id, enabled)
        .justify_between()
        .gap(px(8.0))
        .pr(px(9.0))
        .bg(if open { theme::frame_active() } else { theme::frame() })
        .border_color(if open { theme::accent_main() } else { theme::border() })
        .text_color(theme::text())
        .child(div().overflow_hidden().text_ellipsis().child(label.into()))
        .child(chevron(open, theme::text_dim()))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::frame_hover())))
}

/// One entry in an open dropdown or context menu.
pub fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
    on_click: ClickHandler,
) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(CHIP_RADIUS))
        .whitespace_nowrap()
        .text_color(if selected {
            theme::accent_main()
        } else if enabled {
            theme::text()
        } else {
            theme::text_dim()
        })
        .when(selected, |d| d.bg(theme::accent_tint()))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::frame_hover())).on_click(on_click))
        .child(label.into())
}

/// The box an open dropdown or context menu sits in.
pub fn menu_panel(min_width: Pixels) -> Div {
    div()
        .min_w(min_width)
        .max_h(px(360.0))
        .p(px(4.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .bg(theme::panel())
        .border_1()
        .border_color(theme::border())
        .rounded(px(CONTROL_RADIUS + 2.0))
        .shadow(vec![BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.6),
            offset: point(px(0.0), px(6.0)),
            blur_radius: px(16.0),
            spread_radius: px(0.0),
        }])
        .font_family(theme::FONT_FAMILY)
        .text_size(px(TEXT_SIZE))
}

/// A thin slider: track, accent fill up to `t` (0..1) and a round grab ringed in the
/// panel colour so it stands off the track. The value is shown beside it by the
/// caller, never inside it.
pub fn slider_visual(t: f32, active: bool, enabled: bool) -> Div {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let grab = if active { 16.0 } else { 14.0 };
    let fill: Hsla = if active { theme::accent_hover() } else { theme::accent_main() };
    div()
        .relative()
        .w_full()
        .h(px(4.0))
        .rounded(px(2.0))
        .bg(theme::frame_hover())
        .child(div().absolute().left_0().top_0().h_full().w(gpui::relative(t)).rounded(px(2.0)).bg(fill))
        .child(
            div()
                .absolute()
                .top(px(2.0 - grab / 2.0))
                .left(gpui::relative(t))
                .ml(px(-grab / 2.0))
                .size(px(grab))
                .rounded(px(grab / 2.0))
                .border_2()
                .border_color(theme::panel())
                .bg(fill)
                .when(active, |d| d.shadow(glow(1.5))),
        )
        .when(!enabled, |d| d.opacity(DISABLED))
}

/// A square button with a drawn arrow, for the hold-to-move pad.
pub fn arrow_button(
    id: impl Into<ElementId>,
    arrow: Arrow,
    held: bool,
    size: f32,
    enabled: bool,
    on_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let colour = if held { theme::accent_hover() } else { theme::text() };
    div()
        .id(id)
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .bg(if held { theme::accent_tint() } else { theme::frame() })
        .border_color(if held { theme::accent_main() } else { theme::border() })
        .when(held, |d| d.shadow(glow(1.0)))
        .child(triangle(arrow, 12.0, colour))
        .when(!enabled, |d| d.opacity(DISABLED))
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(theme::frame_hover())).on_mouse_down(MouseButton::Left, on_down)
        })
}

/// The status dot on its own, for a narrow header: an 8 px dot on a 14 px soft disc
/// in a 22 px box, all even steps, so it sits dead centre. No outline to line up.
pub fn status_dot(colour: Hsla) -> Div {
    div().flex().flex_none().items_center().justify_center().size(px(22.0)).child(
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(14.0))
            .rounded(px(7.0))
            .bg(colour.alpha(0.18))
            .child(div().size(px(8.0)).rounded(px(4.0)).bg(colour)),
    )
}

/// The status pill beside the wordmark: 22 px tall with a 1 px border, so its 6 px
/// dot sits exactly 7 px from the top and bottom. `scale` is the window's scale factor.
pub fn status_pill(text: &str, colour: Hsla, scale: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(22.0))
        .px(px(9.0))
        .rounded(px(11.0))
        .bg(colour.alpha(0.10))
        .border_1()
        .border_color(colour.alpha(0.55))
        .text_color(colour)
        .text_size(px(SMALL_SIZE))
        .child(div().flex_none().size(px(6.0)).rounded(px(3.0)).bg(colour))
        // At 100% scale GPUI's pixel snapping leaves the text almost a pixel above the
        // centred dot; at other scales it lands within half a pixel. Measured, so only
        // 100% gets the half-pixel nudge.
        .child(div().relative().top(px(if scale < 1.125 { 0.5 } else { 0.0 })).child(text.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

/// A filled triangle pointing one way, drawn rather than taken from the font.
pub fn triangle(arrow: Arrow, size: f32, colour: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let c = bounds.center();
            let r = px(size * 0.5);
            let s = px(size * 0.3);
            let points = match arrow {
                Arrow::Up => [point(c.x, c.y - r), point(c.x + r, c.y + s), point(c.x - r, c.y + s)],
                Arrow::Down => [point(c.x - r, c.y - s), point(c.x + r, c.y - s), point(c.x, c.y + r)],
                Arrow::Left => [point(c.x - r, c.y), point(c.x + s, c.y - r), point(c.x + s, c.y + r)],
                Arrow::Right => [point(c.x + r, c.y), point(c.x - s, c.y + r), point(c.x - s, c.y - r)],
            };
            let mut builder = PathBuilder::fill();
            builder.add_polygon(&points, true);
            if let Ok(path) = builder.build() {
                window.paint_path(path, colour);
            }
        },
    )
    .flex_none()
    .size(px(size))
}

/// A crescent moon (the outer circle minus an offset inner one), for the sleep button.
pub fn moon(colour: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let c = bounds.center();
            let p = |x: f32, y: f32| point(c.x + px(x), c.y + px(y));
            // Where an r = 6 circle at the centre meets an r = 5 circle at (3, -2.5).
            let (top, bottom) = (p(-0.6, -5.97), p(5.76, 1.67));
            let mut builder = PathBuilder::fill();
            builder.move_to(top);
            builder.arc_to(point(px(6.0), px(6.0)), px(0.0), true, false, bottom);
            builder.arc_to(point(px(5.0), px(5.0)), px(0.0), false, true, top);
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, colour);
            }
        },
    )
    .flex_none()
    .size(px(14.0))
}

/// A square button holding an icon, with its meaning in the tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: impl IntoElement,
    enabled: bool,
    on_click: Option<ClickHandler>,
) -> Stateful<Div> {
    button(id, "", enabled, on_click).w(px(CONTROL_HEIGHT)).px(px(0.0)).child(icon)
}

/// A thin chevron for dropdowns: down when closed, up when open.
pub fn chevron(open: bool, colour: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let c = bounds.center();
            let w = px(4.0);
            let h = px(if open { -2.0 } else { 2.0 });
            let mut builder = PathBuilder::stroke(px(1.25));
            builder.move_to(point(c.x - w, c.y - h));
            builder.line_to(point(c.x, c.y + h));
            builder.line_to(point(c.x + w, c.y - h));
            if let Ok(path) = builder.build() {
                window.paint_path(path, colour);
            }
        },
    )
    .flex_none()
    .size(px(12.0))
}
