//! The app draws its own title bar: the header row doubles as it, with minimise,
//! maximise and close at its right end.
//!
//! - Windows: GPUI hides the system caption (`appears_transparent`) and asks the app
//!   which part of the window is under the mouse, so dragging, double-click to
//!   maximise, Aero Snap and Snap Layouts all stay native.
//! - Linux: the window asks for client-side decorations. The app then draws a soft
//!   shadow and handles the resize edges itself. When the desktop draws its own frame
//!   anyway (some KDE setups, or X11 without a compositor) the buttons are left out.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, point, px, App, Bounds, BoxShadow, ClickEvent, CursorStyle, Decorations, Div, Hitbox, HitboxBehavior,
    MouseButton, PathBuilder, Pixels, Point, ResizeEdge, Size, Stateful, Tiling, Window, WindowControlArea,
};

use super::theme;

/// Height of the bar (the header row).
pub const BAR_HEIGHT: f32 = 44.0;
const BUTTON_WIDTH: f32 = 42.0;
/// Room around a client-decorated Linux window for its shadow and resize edges.
const LINUX_INSET: f32 = 10.0;
/// Corner rounding of a client-decorated Linux window.
pub const ROUNDING: f32 = 8.0;

/// How the window is framed this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The app draws the window buttons.
    pub own_bar: bool,
    /// Linux client-side decorations, with the edges the desktop has tiled.
    pub client: Option<Tiling>,
}

impl Frame {
    pub fn of(window: &Window) -> Frame {
        if cfg!(windows) {
            return Frame { own_bar: true, client: None };
        }
        match window.window_decorations() {
            Decorations::Client { tiling } => Frame { own_bar: true, client: Some(tiling) },
            Decorations::Server => Frame { own_bar: false, client: None },
        }
    }

    /// The invisible margin on the left and right together (Linux client-side
    /// decorations only; a tiled edge has none).
    pub fn horizontal_inset(&self) -> f32 {
        match self.client {
            Some(tiling) => LINUX_INSET * (!tiling.left as u8 + !tiling.right as u8) as f32,
            None => 0.0,
        }
    }

    /// Whether the window's top-right corner is rounded, so the close button follows it.
    pub fn round_top_right(&self) -> bool {
        self.client.is_some_and(|t| !(t.top || t.right))
    }
}

/// Makes the empty parts of the header move the window. Call it before adding the
/// header's children. Interactive children must `.occlude()` so a click on them isn't
/// also taken as a drag.
pub fn drag_area(bar: Stateful<Div>, frame: Frame) -> Stateful<Div> {
    if !frame.own_bar {
        return bar;
    }
    if cfg!(windows) {
        // A layer behind the children. It starts a few pixels down because GPUI asks
        // for the drag area before the resize edges, and the top edge must still resize.
        return bar.relative().child(
            div().absolute().top(px(4.0)).bottom_0().left_0().right_0().window_control_area(WindowControlArea::Drag),
        );
    }
    // The move starts on the first mouse movement after the press, so a
    // double-click still reaches `on_click` and maximises.
    let pressed = Rc::new(Cell::new(false));
    let (down, moved, up) = (pressed.clone(), pressed.clone(), pressed);
    bar.on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
        .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
        .on_mouse_move(move |event, window, _| {
            // Checks the button too: the release may have happened outside the bar.
            if moved.replace(false) && event.pressed_button == Some(MouseButton::Left) {
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| window.show_window_menu(event.position))
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Minimise,
    Maximise,
    Restore,
    Close,
}

/// Minimise, maximise (or restore) and close, flush with the top-right corner.
/// `on_close` runs on Linux; on Windows the close goes through the system, which
/// asks the window's should-close handler as usual.
pub fn window_buttons(
    window: &Window,
    frame: Frame,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let controls = window.window_controls();
    let maximised = window.is_maximized();
    let button = |id: &'static str, glyph: Glyph, area: WindowControlArea| {
        let close = glyph == Glyph::Close;
        div()
            .id(id)
            .flex_none()
            .w(px(BUTTON_WIDTH))
            .h(px(BAR_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .when(close && frame.round_top_right(), |d| d.rounded_tr(px(ROUNDING)))
            .hover(move |s| if close { s.bg(theme::bad()) } else { s.bg(theme::frame_hover()) })
            .active(move |s| if close { s.bg(theme::bad()).opacity(0.85) } else { s.bg(theme::frame_active()) })
            .relative()
            .child(glyph_canvas(glyph))
            .when(cfg!(windows), |d| {
                // Like the drag layer, it leaves the top edge free for resizing.
                d.child(div().absolute().top(px(4.0)).bottom_0().left_0().right_0().window_control_area(area))
            })
    };
    let mut row = div().flex().flex_none().h(px(BAR_HEIGHT)).occlude();
    if controls.minimize {
        row = row.child(
            button("win-min", Glyph::Minimise, WindowControlArea::Min)
                .when(!cfg!(windows), |d| d.on_click(|_, window, _| window.minimize_window())),
        );
    }
    if controls.maximize {
        let glyph = if maximised { Glyph::Restore } else { Glyph::Maximise };
        row = row.child(
            button("win-max", glyph, WindowControlArea::Max)
                .when(!cfg!(windows), |d| d.on_click(|_, window, _| window.zoom_window())),
        );
    }
    row.child(
        button("win-close", Glyph::Close, WindowControlArea::Close).when(!cfg!(windows), |d| d.on_click(on_close)),
    )
}

/// Thin line glyphs, 10 px across, drawn rather than taken from a font.
fn glyph_canvas(glyph: Glyph) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let c = bounds.center();
            let (x0, y0) = (f32::from(c.x) - 5.0, f32::from(c.y) - 5.0);
            let p = |x: f32, y: f32| point(px(x0 + x), px(y0 + y));
            let mut builder = PathBuilder::stroke(px(1.0));
            match glyph {
                Glyph::Minimise => {
                    builder.move_to(p(0.0, 5.0));
                    builder.line_to(p(10.0, 5.0));
                }
                Glyph::Maximise => rect(&mut builder, p(0.5, 0.5), p(9.5, 9.5)),
                Glyph::Restore => {
                    rect(&mut builder, p(0.5, 2.5), p(7.5, 9.5));
                    builder.move_to(p(2.5, 2.5));
                    builder.line_to(p(2.5, 0.5));
                    builder.line_to(p(9.5, 0.5));
                    builder.line_to(p(9.5, 7.5));
                    builder.line_to(p(7.5, 7.5));
                }
                Glyph::Close => {
                    builder.move_to(p(0.0, 0.0));
                    builder.line_to(p(10.0, 10.0));
                    builder.move_to(p(10.0, 0.0));
                    builder.line_to(p(0.0, 10.0));
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, theme::text());
            }
        },
    )
    .size(px(12.0))
}

fn rect(builder: &mut PathBuilder, a: Point<Pixels>, b: Point<Pixels>) {
    builder.move_to(a);
    builder.line_to(point(b.x, a.y));
    builder.line_to(b);
    builder.line_to(point(a.x, b.y));
    builder.close();
}

/// Wraps the app in its Linux client-side frame: a transparent margin with a shadow
/// and resize edges, then the rounded, bordered window. Elsewhere it returns the
/// content as it is.
pub fn client_frame(content: Stateful<Div>, frame: Frame, window: &mut Window) -> Stateful<Div> {
    let Some(tiling) = frame.client else { return content };
    // Tiled edges have no margin; the inset tells the desktop where the window really is.
    window.set_client_inset(px(LINUX_INSET));
    let rounding = px(ROUNDING);
    let content = content
        .border_color(theme::border())
        .when(!tiling.top, |d| d.border_t_1())
        .when(!tiling.bottom, |d| d.border_b_1())
        .when(!tiling.left, |d| d.border_l_1())
        .when(!tiling.right, |d| d.border_r_1())
        .when(!(tiling.top || tiling.left), |d| d.rounded_tl(rounding))
        .when(!(tiling.top || tiling.right), |d| d.rounded_tr(rounding))
        .when(!(tiling.bottom || tiling.left), |d| d.rounded_bl(rounding))
        .when(!(tiling.bottom || tiling.right), |d| d.rounded_br(rounding))
        .when(!tiling.is_tiled(), |d| {
            d.shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.45),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(LINUX_INSET / 2.0),
                spread_radius: px(0.0),
            }])
        })
        // Inside the window the edges don't resize, so the cursor goes back to normal.
        .cursor(CursorStyle::Arrow);
    let edge_size = px(LINUX_INSET);
    div()
        .id("client-frame")
        .size_full()
        .when(!tiling.top, |d| d.pt(edge_size))
        .when(!tiling.bottom, |d| d.pb(edge_size))
        .when(!tiling.left, |d| d.pl(edge_size))
        .when(!tiling.right, |d| d.pr(edge_size))
        .child(
            canvas(
                |_, window, _| {
                    window.insert_hitbox(
                        Bounds::new(point(px(0.0), px(0.0)), window.window_bounds().get_bounds().size),
                        HitboxBehavior::Normal,
                    )
                },
                move |_, hitbox: Hitbox, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(window.mouse_position(), edge_size, size, tiling) {
                        window.set_cursor_style(edge_cursor(edge), &hitbox);
                    }
                },
            )
            .size_full()
            .absolute()
            .top_0()
            .left_0(),
        )
        .on_mouse_move(move |event, window, _| {
            // Repaint only near the edges, where the cursor changes.
            let size = window.window_bounds().get_bounds().size;
            if resize_edge(event.position, edge_size + px(4.0), size, tiling).is_some() {
                window.refresh();
            }
        })
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(event.position, edge_size, size, tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(content)
}

fn edge_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Which edge or corner of the window a point is on, if any. Tiled edges don't resize.
fn resize_edge(pos: Point<Pixels>, edge: Pixels, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    let top = !tiling.top && pos.y < edge;
    let bottom = !tiling.bottom && pos.y > size.height - edge;
    let left = !tiling.left && pos.x < edge;
    let right = !tiling.right && pos.x > size.width - edge;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}
