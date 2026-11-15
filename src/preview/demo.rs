//! The demo camera's preview: a drawn scene (layered waves at dusk), so the app can be
//! tried and screenshotted without a camera.

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

type Rgb = [f32; 3];

/// A colour written as 0xRRGGBB (sRGB), in linear light for blending.
fn lin(hex: u32) -> Rgb {
    let c = |shift: u32| {
        let v = ((hex >> shift) & 0xff) as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [c(16), c(8), c(0)]
}

fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One rolling wave layer: a sum of gentle sines across the frame.
struct Wave {
    base: f32,
    sines: [(f32, f32, f32); 3],
    colour: u32,
    /// Edge softness, in frame heights: the far layers are hazier.
    soft: f32,
    /// How much of the low sun's warmth reaches the crest.
    warmth: f32,
}

/// Back to front. Positions are in frame heights from the top; x runs in frame heights
/// from the centre, so the picture scales with the frame.
static WAVES: [Wave; 5] = [
    Wave {
        base: 0.58,
        sines: [(0.030, 2.1, 0.4), (0.012, 5.3, 1.9), (0.004, 11.0, 0.3)],
        colour: 0x7A5276,
        soft: 0.006,
        warmth: 0.9,
    },
    Wave {
        base: 0.645,
        sines: [(0.034, 1.7, 2.6), (0.014, 4.4, 0.2), (0.005, 9.5, 4.1)],
        colour: 0x4A3A6C,
        soft: 0.004,
        warmth: 0.6,
    },
    Wave {
        base: 0.72,
        sines: [(0.038, 1.4, 5.0), (0.015, 3.9, 3.3), (0.005, 8.7, 1.2)],
        colour: 0x2B3463,
        soft: 0.003,
        warmth: 0.4,
    },
    Wave {
        base: 0.81,
        sines: [(0.040, 1.2, 1.1), (0.016, 3.1, 5.6), (0.006, 7.9, 2.4)],
        colour: 0x153252,
        soft: 0.002,
        warmth: 0.25,
    },
    Wave {
        base: 0.91,
        sines: [(0.036, 1.0, 3.8), (0.014, 2.7, 0.9), (0.005, 6.8, 5.2)],
        colour: 0x0A2036,
        soft: 0.0015,
        warmth: 0.15,
    },
];

/// Where the low sun sits (x from the centre, y from the top, both in frame heights).
const SUN: (f32, f32) = (0.30, 0.56);

/// The picture is soft throughout, so it is drawn at no more than this many rows and scaled up.
const DRAW_ROWS: usize = 360;

/// The picture, drawn small (as sRGB levels, 0..255) and scaled up as each frame is made;
/// frames differ only in grain.
struct Scene {
    width: usize,
    small: Vec<Rgb>,
    small_width: usize,
    /// For each output column (and row): the source column (row) and the weight of the next.
    cols: Vec<(usize, f32)>,
    rows: Vec<(usize, f32)>,
}

impl Scene {
    fn new(width: u32, height: u32) -> Self {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        let lh = h.min(DRAW_ROWS);
        let lw = ((w * lh + h / 2) / h).max(1);
        let taps = |out: usize, src: usize| -> Vec<(usize, f32)> {
            (0..out)
                .map(|i| {
                    let pos = ((i as f32 + 0.5) * src as f32 / out as f32 - 0.5).clamp(0.0, (src - 1) as f32);
                    let i0 = (pos as usize).min(src.saturating_sub(2));
                    (i0, (pos - i0 as f32).min(1.0))
                })
                .collect()
        };
        Scene { width: w, small: draw(lw, lh), small_width: lw, cols: taps(w, lw), rows: taps(h, lh) }
    }

    /// One BGRA frame. `phase` (0..1) changes the fine grain, so the picture looks live and
    /// the smooth gradients do not band.
    fn frame(&self, phase: f32) -> Vec<u8> {
        // About one and a half levels of grain either way (plus a half, so the casts below
        // round), from a short table whose length does not line up with the rows.
        let mut seed = 0x9E37_79B9u32 ^ (phase.fract() * 4096.0) as u32;
        let grain: Vec<f32> = (0..GRAIN_LEN)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed >> 8) as f32 / (1u32 << 24) as f32 * 3.0 - 1.0
            })
            .collect();
        let (w, lw) = (self.width, self.small_width);
        let mut out = vec![0u8; w * self.rows.len() * 4];
        let mut line = vec![[0.0f32; 3]; lw];
        let mut g = 0usize;
        for (row, &(y0, fy)) in out.chunks_exact_mut(w * 4).zip(&self.rows) {
            let (above, below) =
                (&self.small[y0 * lw..][..lw], &self.small[((y0 + 1) * lw).min(self.small.len() - lw)..][..lw]);
            for ((l, a), b) in line.iter_mut().zip(above).zip(below) {
                *l = mix3(*a, *b, fy);
            }
            for (o, &(x0, fx)) in row.chunks_exact_mut(4).zip(&self.cols) {
                let c = mix3(line[x0], line[(x0 + 1).min(lw - 1)], fx);
                let n = grain[g];
                g = if g + 1 == GRAIN_LEN { 0 } else { g + 1 };
                // Float to integer casts saturate, so no clamping is needed.
                o.copy_from_slice(&[(c[2] + n) as u8, (c[1] + n) as u8, (c[0] + n) as u8, 255]);
            }
        }
        out
    }
}

/// The length of the grain table: prime, so the pattern does not repeat in columns.
const GRAIN_LEN: usize = 4099;

/// Rolling layered waves at dusk under a softly glowing sky, as sRGB levels (0..255).
fn draw(w: usize, h: usize) -> Vec<Rgb> {
    let hf = h as f32;
    // Sky colours, each blended in over a band of heights that overlaps the next.
    let sky_blends = [
        (0.0, 0.0, lin(0x0C1232)),
        (0.0, 0.45, lin(0x272558)),
        (0.20, 0.56, lin(0x7C4A7E)),
        (0.40, 0.66, lin(0xEFA176)),
    ];
    let sun_core = lin(0xFFE2B8);
    let glow = lin(0xFFB27A);
    let crest = lin(0x67F7FF);
    let colours: Vec<Rgb> = WAVES.iter().map(|wave| lin(wave.colour)).collect();
    // Linear light to sRGB levels, in 1/4096 steps.
    let lut: Vec<f32> = (0..4096)
        .map(|i| {
            let v = i as f32 / 4095.0;
            let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            s * 255.0
        })
        .collect();

    // Per column: each wave's height and how close the column is to the sun.
    let xs: Vec<f32> = (0..w).map(|x| (x as f32 + 0.5 - w as f32 / 2.0) / hf).collect();
    let heights: Vec<[f32; 5]> = xs
        .iter()
        .map(|&u| {
            let mut ys = [0.0; 5];
            for (y, wave) in ys.iter_mut().zip(&WAVES) {
                *y = wave.base + wave.sines.iter().map(|&(a, f, p)| a * (u * f * 3.0 + p).sin()).sum::<f32>();
            }
            ys
        })
        .collect();
    let sun_near: Vec<f32> = xs.iter().map(|&u| 1.0 / (1.0 + ((u - SUN.0) / 0.35).powi(2))).collect();

    // The sky, row by row: deep indigo overhead, through violet and rose, to a peach horizon.
    let sky: Vec<Rgb> = (0..h)
        .map(|y| {
            let v = (y as f32 + 0.5) / hf;
            sky_blends[1..].iter().fold(sky_blends[0].2, |c, &(v0, v1, to)| mix3(c, to, smoothstep(v0, v1, v)))
        })
        .collect();
    // The vignette, as a row factor times a column factor.
    let fade = |t: f32| 1.0 - 0.30 * (2.0 * t - 1.0).powi(2);
    let col_fade: Vec<f32> = (0..w).map(|x| fade((x as f32 + 0.5) / w as f32)).collect();

    // A warm haze along the horizon (by row), strongest near the sun (by column).
    let haze_row: Vec<f32> = (0..h).map(|y| 0.30 / (1.0 + (((y as f32 + 0.5) / hf - SUN.1) / 0.07).powi(2))).collect();
    // The sun's halo fades to nothing at this distance, so it is only worked out nearby.
    const HALO_REACH2: f32 = 0.16;
    let halo_floor = 0.45 / (1.0 + HALO_REACH2 / 0.010);
    // Edges and the crest lines stay at least about a pixel wide, so they never step.
    let softs: Vec<f32> = WAVES.iter().map(|wave| wave.soft.max(0.9 / hf)).collect();
    let line_width = 0.0025f32.max(0.8 / hf);

    let mut pixels = vec![[0.0; 3]; w * h];
    for y in 0..h {
        let v = (y as f32 + 0.5) / hf;
        let row_fade = fade(v);
        let dv = v - SUN.1;
        for x in 0..w {
            let ys = &heights[x];
            let near = sun_near[x];
            // Start from the nearest layer that hides everything behind it.
            let mut first = WAVES.len();
            while first > 0 && v < ys[first - 1] + softs[first - 1] {
                first -= 1;
            }
            let (mut c, start) = if first > 0 {
                ([0.0; 3], first - 1)
            } else {
                let halo = haze_row[y] * near + {
                    let du = xs[x] - SUN.0;
                    let d2 = du * du + dv * dv;
                    if d2 < HALO_REACH2 {
                        0.45 / (1.0 + d2 * 100.0) - halo_floor
                    } else {
                        0.0
                    }
                };
                let mut c = sky[y];
                for i in 0..3 {
                    c[i] += glow[i] * halo;
                }
                // The sun's disc.
                let du = xs[x] - SUN.0;
                let d2 = du * du + dv * dv;
                if d2 < 0.0028 {
                    let disc = 1.0 - smoothstep(0.047, 0.052, d2.sqrt());
                    c = mix3(c, sun_core, disc);
                }
                (c, 0)
            };
            // How much of the pixel so far is wave rather than sky.
            let mut solid = if first > 0 { 1.0f32 } else { 0.0 };
            for (k, wave) in WAVES.iter().enumerate().skip(start) {
                let top = ys[k];
                if v < top - 0.06 {
                    continue;
                }
                // Each layer casts a soft shadow on the one behind it (not on the sky).
                if solid > 0.0 && k > start {
                    let shade = 1.0 - 0.30 * solid * smoothstep(top - 0.06, top, v);
                    for ch in c.iter_mut() {
                        *ch *= shade;
                    }
                }
                let soft = softs[k];
                let cover = smoothstep(top - soft, top + soft, v);
                if cover <= 0.0 {
                    continue;
                }
                // Lit at the crest, falling into shadow below; warmer facing the sun.
                let depth = (v - top).max(0.0);
                let t = depth * (1.0 / 0.05);
                let fall = 1.0 - 0.45 * smoothstep(0.0, 0.25, depth);
                let lit = fall * (1.0 + 0.9 / (1.0 + t + t * t) * (0.4 + wave.warmth * near));
                let warm = 0.10 * wave.warmth * near * fall;
                let mut col = colours[k];
                for i in 0..3 {
                    col[i] = col[i] * lit + glow[i] * warm;
                }
                // A fine line of cool light just inside the near crests.
                let d = (v - top - soft) / line_width;
                if d.abs() < 12.0 {
                    let line = 0.22 * (1.0 - wave.warmth) * (1.0 - near * 0.6) / (1.0 + d * d);
                    col = mix3(col, crest, line);
                }
                c = mix3(c, col, cover);
                solid += (1.0 - solid) * cover;
            }
            let k = row_fade * col_fade[x];
            // Stored as sRGB levels (0..255), ready for the grain.
            pixels[y * w + x] = c.map(|ch| lut[((ch * k).clamp(0.0, 1.0) * 4095.0) as usize]);
        }
    }
    // A scatter of faint stars high in the sky, about two pixels across at any size.
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let r = (hf / 520.0).max(0.8);
    for _ in 0..40 {
        let (sx, sy, bright) = (next() * w as f32, next() * next() * 0.42 * hf, next());
        let strength = (0.35 + 0.65 * bright * bright) * 150.0 * (1.0 - smoothstep(0.12, 0.42, sy / hf));
        let reach = (r * 3.0).ceil() as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (px, py) = (sx as i32 + dx, sy as i32 + dy);
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    continue;
                }
                let (fx, fy) = (px as f32 + 0.5 - sx, py as f32 + 0.5 - sy);
                let a = strength / (1.0 + (fx * fx + fy * fy) / (r * r)).powi(3);
                for ch in pixels[py as usize * w + px as usize].iter_mut() {
                    *ch += a;
                }
            }
        }
    }
    pixels
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
    let scene = Scene::new(width, height);
    let frames: Vec<Vec<u8>> = (0..4).map(|i| scene.frame(i as f32 / 4.0)).collect();
    drop(scene);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_fills_every_size() {
        for f in formats() {
            let frame = Scene::new(f.width, f.height).frame(0.25);
            assert_eq!(frame.len(), f.width as usize * f.height as usize * 4);
            assert!(frame.chunks_exact(4).all(|p| p[3] == 255));
        }
    }
}
