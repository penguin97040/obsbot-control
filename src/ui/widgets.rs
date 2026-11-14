//! The small widget kit every control row uses, so things line up across sections:
//!   label | control (stretches) | value (right-aligned) | trailing (Auto switch)
//! Each builder takes ready-made event handlers (see `Context::listener`).

use gpui::prelude::*;
use gpui::{
    canvas, div, linear_color_stop, linear_gradient, point, px, AnyView, App, Bounds, BoxShadow, ClickEvent, Div,
    ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent, PathBuilder, Pixels, SharedString, Stateful, Window,
};

use super::theme::{self, CONTROL_HEIGHT, LABEL_WIDTH, UNIT_WIDTH, VALUE_WIDTH};

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A soft neon halo for selected and hovered controls.
pub fn glow(strength: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme::accent(0.18 * strength),
        offset: gpui::point(px(0.0), px(0.0)),
        blur_radius: px(8.0),
        spread_radius: px(0.0),
    }]
}

struct TooltipView(SharedString);

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(380.0))
            .px(px(10.0))
            .py(px(6.0))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .rounded(px(3.0))
            .text_color(theme::text())
            .text_size(px(13.0))
            .font_family(theme::FONT_FAMILY)
            .child(self.0.clone())
    }
}

/// A tooltip builder for `.tooltip(...)`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| TooltipView(text.clone())).into()
}

/// A heading in the bold face and accent colour over a thin line that fades out.
pub fn section_title(text: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .pt(px(10.0))
        .pb(px(2.0))
        .child(
            div()
                .text_color(theme::accent_main())
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(14.0))
                .child(text.to_uppercase()),
        )
        .child(div().h(px(1.0)).w_full().bg(linear_gradient(
            90.0,
            linear_color_stop(theme::accent_dim(), 0.0),
            linear_color_stop(theme::accent(0.0), 1.0),
        )))
}

/// Dim, smaller explanatory text.
pub fn note(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::text_dim()).text_size(px(13.0)).child(text.into())
}

/// The label column of a row (with an optional "(?)" help tooltip).
pub fn label_cell(id: impl Into<ElementId>, label: &str, width: f32, help: Option<&'static str>) -> Stateful<Div> {
    let cell = div()
        .id(id)
        .flex_none()
        .w(px(width))
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(div().whitespace_nowrap().child(label.to_string()));
    match help {
        Some(text) => {
            cell.child(div().text_color(theme::text_dim()).text_size(px(12.0)).child("(?)")).tooltip(tip(text))
        }
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
        .child(div().w(px(VALUE_WIDTH)).flex().justify_end().child(number))
        .when(unit_slot, |d| {
            d.child(div().w(px(UNIT_WIDTH)).pl(px(1.0)).text_color(theme::text_dim()).child(unit.to_string()))
        })
}

fn base_button(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(CONTROL_HEIGHT))
        .px(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .whitespace_nowrap()
        .when(!enabled, |d| d.opacity(0.55))
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

/// A button that shows whether it is selected ("neon outline" when it is).
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

/// A row of joined options, one of them selected (or none, when `selected` is None).
/// `on_select` receives the index clicked.
pub fn segmented(
    id: &str,
    options: &[&str],
    tips: &[&'static str],
    selected: Option<usize>,
    enabled: bool,
    on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = std::rc::Rc::new(on_select);
    let mut row = div().flex().flex_1().min_w(px(0.0)).h(px(CONTROL_HEIGHT));
    let count = options.len();
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
            .whitespace_nowrap()
            .overflow_hidden()
            .child(option.to_string());
        // Cells share their dividers instead of overlapping, so the selected cell's
        // outline is never painted over by its neighbour. Each cell draws its top,
        // bottom and left edges; the last also draws the right one. A divider takes
        // the accent when either cell beside it is selected.
        let accent_edge = |selected: bool| if selected { theme::accent_main() } else { theme::border() };
        let left_selected = i > 0 && selected == Some(i - 1);
        cell = cell
            .border_t_1()
            .border_b_1()
            .border_l_1()
            .border_color(accent_edge(is_selected))
            .when(i == 0, |c| c.rounded_l(px(3.0)))
            .when(i + 1 == count, |c| c.border_r_1().rounded_r(px(3.0)))
            .when(i > 0, |c| {
                // The shared divider on the left, drawn over the cell's own left edge.
                c.relative().child(
                    div()
                        .absolute()
                        .left(px(-1.0))
                        .top(px(-1.0))
                        .bottom(px(-1.0))
                        .w(px(1.0))
                        .bg(accent_edge(is_selected || left_selected)),
                )
            });
        cell = if is_selected {
            cell.bg(theme::accent_tint()).text_color(theme::accent_main())
        } else {
            cell.bg(theme::frame()).text_color(theme::text())
        };
        if !enabled {
            cell = cell.opacity(0.55);
        } else {
            cell = cell
                .cursor_pointer()
                .hover(
                    move |s| if is_selected { s.text_color(theme::accent_hover()) } else { s.bg(theme::frame_hover()) },
                )
                .on_click(move |_, window, cx| handler(&i, window, cx));
        }
        if let Some(text) = tips.get(i) {
            cell = cell.tooltip(tip(*text));
        }
        row = row.child(cell);
    }
    row
}

/// An on/off switch with an optional label after it.
pub fn toggle_switch(
    id: impl Into<ElementId>,
    label: Option<&str>,
    on: bool,
    enabled: bool,
    on_toggle: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let height = 18.0;
    let width = 32.0;
    let knob = height - 6.0;
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
                .bg(if on { theme::accent_main() } else { theme::text_dim() }),
        );
    let mut el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(CONTROL_HEIGHT))
        .child(track)
        .when_some(label, |d, text| d.child(text.to_string()))
        .when(!enabled, |d| d.opacity(0.55));
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
        .py(px(5.0))
        .rounded(px(2.0))
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
        .bg(theme::panel())
        .border_1()
        .border_color(theme::accent_dim())
        .rounded(px(3.0))
        .shadow(vec![BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.6),
            offset: gpui::point(px(0.0), px(4.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
        }])
        .font_family(theme::FONT_FAMILY)
        .text_size(px(theme::TEXT_SIZE))
}

/// A thin slider: track, accent fill up to `t` (0..1) and a round grab. The value is
/// shown beside it by the caller, never inside it.
pub fn slider_visual(t: f32, active: bool, enabled: bool) -> Div {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let grab = if active { 15.0 } else { 13.0 };
    let fill: Hsla = if active { theme::accent_hover() } else { theme::accent_main() };
    div()
        .relative()
        .w_full()
        .h(px(4.0))
        .rounded(px(2.0))
        .bg(if active { theme::frame_active() } else { theme::frame_hover() })
        .child(div().absolute().left_0().top_0().h_full().w(gpui::relative(t)).rounded(px(2.0)).bg(fill))
        .child(
            div()
                .absolute()
                .top(px(2.0 - grab / 2.0))
                .left(gpui::relative(t))
                .ml(px(-grab / 2.0))
                .size(px(grab))
                .rounded(px(grab / 2.0))
                .bg(fill)
                .when(active, |d| d.shadow(glow(1.5))),
        )
        .when(!enabled, |d| d.opacity(0.55))
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
    div()
        .id(id)
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .bg(if held { theme::frame_active() } else { theme::frame() })
        .border_color(if held { theme::accent_main() } else { theme::border() })
        .text_color(if held { theme::accent_hover() } else { theme::text() })
        .when(held, |d| d.shadow(glow(1.0)))
        .child(triangle(arrow, size * 0.36, if held { theme::accent_hover() } else { theme::text() }))
        .when(!enabled, |d| d.opacity(0.55))
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(theme::frame_hover())).on_mouse_down(MouseButton::Left, on_down)
        })
}

/// The status pill's tinted, outlined shape around its dot.
fn status_shape(colour: Hsla) -> Div {
    let mut tint = colour;
    tint.a = 0.12;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(px(24.0))
        .rounded(px(12.0))
        .bg(tint)
        .border_1()
        .border_color(colour)
        .child(div().size(px(7.0)).rounded(px(4.0)).bg(colour))
}

/// The status pill's dot on its own, for a narrow header.
pub fn status_dot(colour: Hsla) -> Div {
    status_shape(colour).w(px(24.0))
}

/// The status pill beside the wordmark.
pub fn status_pill(text: &str, colour: Hsla) -> Div {
    status_shape(colour).px(px(10.0)).text_color(colour).text_size(px(13.0)).child(text.to_string())
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

/// A thin chevron for dropdowns: down when closed, up when open.
pub fn chevron(open: bool, colour: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let c = bounds.center();
            let w = px(4.5);
            let h = px(if open { -2.5 } else { 2.5 });
            let mut builder = PathBuilder::stroke(px(1.5));
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
