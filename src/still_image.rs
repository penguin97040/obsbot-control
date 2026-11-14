//! A small, softened copy of a preview frame, shown dimmed behind "Preview paused".
//! Drawn at full size, the GPU's smooth scaling finishes the blur.

/// Averages a tightly packed BGRA frame down to about `target_width` pixels wide,
/// keeping its shape, then softens it with two 3×3 box blurs. Returns the pixels
/// (BGRA, alpha 255) and their width and height, or None if there is no image.
pub fn blurred_thumbnail(bgra: &[u8], width: u32, height: u32, target_width: u32) -> Option<(Vec<u8>, u32, u32)> {
    if width == 0 || height == 0 || target_width == 0 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    // Checked, so a huge size on a 32-bit target cannot wrap round and pass.
    let needed = w.checked_mul(h)?.checked_mul(4)?;
    if bgra.len() < needed {
        return None;
    }
    let tw = (target_width as usize).min(w);
    let th = (((h as f64) * (tw as f64) / (w as f64)).round() as usize).clamp(1, h);
    let mut img = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let y0 = ty * h / th;
        let y1 = (y0 + 1).max((ty + 1) * h / th);
        for tx in 0..tw {
            let x0 = tx * w / tw;
            let x1 = (x0 + 1).max((tx + 1) * w / tw);
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                let row = &bgra[(y * w + x0) * 4..(y * w + x1) * 4];
                for px in row.chunks_exact(4) {
                    for c in 0..3 {
                        sum[c] += px[c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let out = &mut img[(ty * tw + tx) * 4..][..4];
            for c in 0..3 {
                out[c] = (sum[c] / n) as u8;
            }
            out[3] = 0xFF;
        }
    }
    let mut tmp = vec![0u8; img.len()];
    for _ in 0..2 {
        for y in 0..th {
            for x in 0..tw {
                let mut sum = [0u32; 3];
                let mut n = 0u32;
                for sy in y.saturating_sub(1)..=(y + 1).min(th - 1) {
                    for sx in x.saturating_sub(1)..=(x + 1).min(tw - 1) {
                        let p = &img[(sy * tw + sx) * 4..][..3];
                        for c in 0..3 {
                            sum[c] += p[c] as u32;
                        }
                        n += 1;
                    }
                }
                let q = &mut tmp[(y * tw + x) * 4..][..4];
                for c in 0..3 {
                    q[c] = (sum[c] / n) as u8;
                }
                q[3] = 0xFF;
            }
        }
        std::mem::swap(&mut img, &mut tmp);
    }
    Some((img, tw as u32, th as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, b: u8, g: u8, r: u8) -> Vec<u8> {
        // Alpha is undefined in RGB32 frames; the still must be opaque anyway.
        (0..w * h).flat_map(|_| [b, g, r, 0]).collect()
    }

    #[test]
    fn keeps_shape_and_plain_colour() {
        let f = frame(1920, 1080, 10, 120, 250);
        let (out, w, h) = blurred_thumbnail(&f, 1920, 1080, 48).unwrap();
        assert_eq!((w, h), (48, 27));
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert!(out.chunks_exact(4).all(|p| p == [10, 120, 250, 0xFF]));
    }

    #[test]
    fn bright_block_spreads_and_dims() {
        let mut f = frame(480, 480, 0, 0, 0);
        for y in 230..250 {
            for x in 230..250 {
                f[(y * 480 + x) * 4 + 1] = 255;
            }
        }
        let (out, w, h) = blurred_thumbnail(&f, 480, 480, 48).unwrap();
        assert_eq!((w, h), (48, 48));
        let green = |x: usize, y: usize| out[(y * w as usize + x) * 4 + 1];
        assert!(green(23, 23) > 0 && green(23, 23) < 255);
        assert!(green(21, 23) > 0);
        assert_eq!(green(0, 0), 0);
    }

    #[test]
    fn small_odd_and_tiny_sizes() {
        let (_, w, h) = blurred_thumbnail(&frame(7, 3, 1, 2, 3), 7, 3, 48).unwrap();
        assert_eq!((w, h), (7, 3));
        let (out, w, h) = blurred_thumbnail(&frame(1, 1, 9, 9, 9), 1, 1, 48).unwrap();
        assert_eq!((w, h, out[0]), (1, 1, 9));
        let (_, w, h) = blurred_thumbnail(&frame(3, 2000, 5, 5, 5), 3, 2000, 48).unwrap();
        assert_eq!((w, h), (3, 2000));
    }

    #[test]
    fn bad_input_gives_none() {
        assert!(blurred_thumbnail(&[], 10, 10, 48).is_none());
        assert!(blurred_thumbnail(&[0; 400], 0, 10, 48).is_none());
        assert!(blurred_thumbnail(&[0; 400], 10, 0, 48).is_none());
        assert!(blurred_thumbnail(&[0; 400], 10, 10, 0).is_none());
        // One byte short.
        assert!(blurred_thumbnail(&[0; 399], 10, 10, 4).is_none());
        assert!(blurred_thumbnail(&[0; 400], 10, 10, 4).is_some());
        assert!(blurred_thumbnail(&[0; 16], u32::MAX, u32::MAX, 4).is_none());
    }
}
