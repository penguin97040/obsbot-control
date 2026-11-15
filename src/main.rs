//! obsbot-control: an unofficial, open-source control app for OBSBOT Tiny 2 and
//! Tiny 2 Lite webcams, for Windows and Linux. Works entirely offline.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod camera;
mod camera_use;
mod compact;
mod logx;
mod platform;
mod preview;
mod preview_format;
mod protocol;
mod safety;
mod settings;
mod still_image;
mod ui;
#[cfg(target_os = "linux")]
mod v4l2;

use std::borrow::Cow;

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions,
};

use settings::Settings;
use ui::{AppView, APP_TITLE};

const HELP: &str = "obsbot-control: control an OBSBOT Tiny 2 or Tiny 2 Lite webcam.

Usage: obsbot-control [--demo]

  --demo      Use a simulated camera (nothing is sent to any real camera).
  --version   Show the version.
  --help      Show this help.";

fn main() {
    #[cfg(windows)]
    platform::harden_dll_search();
    // The release exe has no console; attach to the one it was started from only to
    // print and exit, so closing that terminal later can't close the app.
    let console = || {
        #[cfg(windows)]
        platform::attach_parent_console();
    };
    let mut demo = std::env::var_os("OBSBOT_CONTROL_DEMO").is_some_and(|v| v == "1");
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--demo" => demo = true,
            "--version" => {
                console();
                println!("obsbot-control {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                console();
                println!("{HELP}");
                return;
            }
            other => {
                console();
                eprintln!("Unknown option: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    // Only one copy talks to the camera at a time.
    let Some(_instance) = platform::single_instance() else {
        eprintln!("obsbot-control is already running.");
        return;
    };

    let settings = Settings::load();
    // The demo camera leaves the real log (and settings) alone.
    if let Some(dir) = Settings::directory().filter(|_| !demo) {
        logx::open_file(&dir.join("obsbot-control.log"));
    }
    log_info!(
        "obsbot-control {} started on {}{}.",
        env!("CARGO_PKG_VERSION"),
        platform::os_version(),
        if demo { ", with the demo camera" } else { "" }
    );
    if Settings::directory().is_none() {
        log_warn!("The settings folder isn't available, so settings won't be saved.");
    }

    Application::new().run(move |cx: &mut App| {
        let fonts: Vec<Cow<'static, [u8]>> = ui::theme::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
        if let Err(e) = cx.text_system().add_fonts(fonts) {
            log_warn!("Could not load the built-in font: {e}");
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let width = settings.window_w.clamp(400, 4000) as f32;
        let height = settings.window_h.clamp(480, 3000) as f32;
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // The app draws its own title bar (src/ui/titlebar.rs). The title still
            // names the window in the taskbar and for the single-instance check.
            titlebar: Some(TitlebarOptions {
                title: Some(APP_TITLE.into()),
                appears_transparent: true,
                ..Default::default()
            }),
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("obsbot-control".into()),
            window_min_size: Some(size(px(400.0), px(480.0))),
            focus: true,
            show: true,
            ..Default::default()
        };
        let opened = cx.open_window(options, move |window, cx| {
            // Only Windows needs this (a dark title bar). GPUI's X11 windows don't
            // provide a raw handle at all, so don't ask for one elsewhere.
            #[cfg(windows)]
            if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) {
                platform::style_window(handle.as_raw());
            }
            let view = cx.new(|cx| AppView::new(settings, demo, window, cx));
            let closing = view.clone();
            window.on_window_should_close(cx, move |window, cx| {
                closing.update(cx, |view, _| view.shutdown(window));
                true
            });
            view
        });
        match opened {
            Ok(_) => cx.activate(true),
            Err(e) => {
                log_error!("Could not open the window: {e}");
                cx.quit();
            }
        }
    });
}
