//! The demo camera's preview: a drawn scene (soft lights behind a silhouette), so the
//! app can be tried and screenshotted without a camera.

use std::time::{Duration, Instant};

use super::Sink;
use crate::preview_format::{self, PreviewFormat};

/// The DeviceInfo::path of the demo camera.
pub const DEMO_PATH: &str = "demo";

fn formats() -> Vec<PreviewFormat> {
    let f = |width, height, fps_num, fourcc: &str| PreviewFormat {
        width,
        height,
        fps_num,
        fps_den: 1,
        fourcc: fourcc.to_string(),
    };
    vec![
        f(1920, 1080, 30, "MJPG"),
        f(1280, 720, 60, "MJPG"),
        f(1280, 720, 30, "MJPG"),
        f(1280, 720, 10, "YUYV"),
        f(960, 540, 30, "MJPG"),
        f(640, 360, 30, "MJPG"),
        f(640, 360, 30, "YUYV"),
    ]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Draws the scene at this size as BGRA. `phase` (0..1) drifts the background lights.
pub fn render_scene(width: u32, height: u32, phase: f32) -> Vec<u8> {
    let (w, h) = (width as f32, height as f32);
    let mut out = vec![0u8; width as usize * height as usize * 4];
    // Out-of-focus lights behind the subject: (x, y, radius, r, g, b, strength).
    let drift = (phase * std::f32::consts::TAU).sin() * 0.01;
    let lights: [(f32, f32, f32, f32, f32, f32, f32); 7] = [
        (0.12 + drift, 0.22, 0.10, 1.00, 0.72, 0.40, 0.55),
        (0.24, 0.62, 0.07, 0.40, 0.97, 1.00, 0.35),
        (0.80 - drift, 0.18, 0.12, 0.40, 0.97, 1.00, 0.30),
        (0.90, 0.55, 0.08, 1.00, 0.55, 0.70, 0.30),
        (0.66, 0.35, 0.05, 1.00, 0.85, 0.55, 0.45),
        (0.36, 0.14, 0.05, 0.70, 0.80, 1.00, 0.35),
        (0.06, 0.80, 0.09, 0.30, 0.60, 1.00, 0.25),
    ];
    for y in 0..height {
        let fy = y as f32 / h;
        for x in 0..width {
            let fx = x as f32 / w;
            // A dim room: cool at the top, warmer towards a window on the left.
            let mut r = mix(0.05, 0.10, fy) + 0.10 * (1.0 - fx).powi(3);
            let mut g = mix(0.11, 0.09, fy) + 0.06 * (1.0 - fx).powi(3);
            let mut b = mix(0.16, 0.10, fy) + 0.02 * (1.0 - fx).powi(3);
            for &(lx, ly, lr, cr, cg, cb, s) in &lights {
                let dx = (fx - lx) * w / h;
                let dy = fy - ly;
                let d = (dx * dx + dy * dy).sqrt();
                let a = (1.0 - smoothstep(lr * 0.75, lr, d)) * s;
                r += cr * a * 0.6;
                g += cg * a * 0.6;
                b += cb * a * 0.6;
            }
            // The subject: a head and shoulders, with a thin cyan rim light.
            let hx = (fx - 0.5) * w / h;
            let head = ((hx / 0.105).powi(2) + ((fy - 0.43) / 0.14).powi(2)).sqrt();
            let shoulders = ((hx / 0.33).powi(2) + ((fy - 1.02) / 0.36).powi(2)).sqrt();
            let neck = ((hx / 0.055).powi(2) + ((fy - 0.64) / 0.12).powi(2)).sqrt();
            // A smooth union, so the head, neck and shoulders blend into one shape.
            let smin = |a: f32, b: f32, k: f32| {
                let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
                mix(b, a, h) - k * h * (1.0 - h)
            };
            let body = smin(smin(head, neck, 0.25), shoulders, 0.25);
            let inside = 1.0 - smoothstep(0.985, 1.0, body);
            let rim = (smoothstep(0.90, 0.985, body) - smoothstep(0.985, 1.0, body)).max(0.0) * 0.9;
            r = mix(r, 0.045, inside) + rim * 0.40;
            g = mix(g, 0.060, inside) + rim * 0.97;
            b = mix(b, 0.075, inside) + rim * 1.00;
            // Vignette.
            let vx = fx - 0.5;
            let vy = fy - 0.5;
            let v = 1.0 - (vx * vx + vy * vy) * 0.9;
            let i = (y as usize * width as usize + x as usize) * 4;
            out[i] = ((b * v).clamp(0.0, 1.0) * 255.0) as u8;
            out[i + 1] = ((g * v).clamp(0.0, 1.0) * 255.0) as u8;
            out[i + 2] = ((r * v).clamp(0.0, 1.0) * 255.0) as u8;
            out[i + 3] = 255;
        }
    }
    out
}

pub fn run(format_key: &str, sink: &Sink) {
    let list = formats();
    let default = preview_format::best_default(&list).cloned().unwrap_or_else(|| list[0].clone());
    sink.set_formats(list.clone(), default.key());
    let chosen = preview_format::find(&list, format_key).cloned().unwrap_or(default);
    sink.set_current(chosen.label(), chosen.key());
    crate::log_info!("Preview started ({}).", chosen.label());

    // The scene is drawn a few times and cycled, so the demo uses little processor time.
    let (width, height) = (chosen.width, chosen.height);
    let frames: Vec<Vec<u8>> = (0..4).map(|i| render_scene(width, height, i as f32 / 4.0)).collect();
    let interval = Duration::from_secs_f64(1.0 / chosen.fps().clamp(1.0, 60.0));
    let mut next = Instant::now();
    let mut index = 0usize;
    while !sink.stop_requested() {
        sink.publish(width, height, frames[index / 8 % frames.len()].clone());
        index += 1;
        next += interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep((next - now).min(Duration::from_millis(100)));
        } else {
            next = now;
        }
    }
}
