//! Pixel format conversion to tightly packed BGRA, for the Linux preview.
//!
//! YUV uses BT.601 limited range (Y 16–235, U/V 16–240), which is what UVC cameras
//! send, with the usual 8-bit fixed-point coefficients.

use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::safety;

fn clamp_u8(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

/// Writes one BGRA pixel from Y, U (Cb) and V (Cr).
#[inline]
fn yuv_pixel(y: u8, u: u8, v: u8, out: &mut [u8]) {
    let c = 298 * (y as i32 - 16);
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    out[0] = clamp_u8((c + 516 * d + 128) >> 8);
    out[1] = clamp_u8((c - 100 * d - 208 * e + 128) >> 8);
    out[2] = clamp_u8((c + 409 * e + 128) >> 8);
    out[3] = 255;
}

/// Bytes needed for `rows` rows `stride` apart whose last row uses `row_bytes`.
fn bytes_needed(stride: usize, rows: usize, row_bytes: usize) -> Option<usize> {
    stride.checked_mul(rows.checked_sub(1)?)?.checked_add(row_bytes)
}

/// YUYV 4:2:2 (bytes Y0 U Y1 V) with `stride` bytes per row.
pub fn yuyv_to_bgra(src: &[u8], width: u32, height: u32, stride: usize) -> Option<Vec<u8>> {
    if !safety::geometry(width, height) {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    // Each 4-byte group covers two pixels; an odd width still needs a whole group.
    let row_bytes = w.div_ceil(2) * 4;
    if stride < row_bytes || bytes_needed(stride, h, row_bytes)? > src.len() {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    for (row, dst) in out.chunks_exact_mut(w * 4).enumerate() {
        let line = &src[row * stride..row * stride + row_bytes];
        for (pair, group) in dst.chunks_mut(8).zip(line.chunks_exact(4)) {
            let (u, v) = (group[1], group[3]);
            yuv_pixel(group[0], u, v, &mut pair[..4]);
            if pair.len() == 8 {
                yuv_pixel(group[2], u, v, &mut pair[4..]);
            }
        }
    }
    Some(out)
}

/// NV12: a full-size Y plane then an interleaved half-size UV plane, `stride` bytes per row.
pub fn nv12_to_bgra(src: &[u8], width: u32, height: u32, stride: usize) -> Option<Vec<u8>> {
    if !safety::geometry(width, height) {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let chroma_rows = h.div_ceil(2);
    let chroma_row_bytes = w.div_ceil(2) * 2;
    if stride < w || stride < chroma_row_bytes {
        return None;
    }
    // V4L2's single-plane NV12 puts the UV plane straight after the Y plane, with
    // the same stride.
    let y_size = stride.checked_mul(h)?;
    if y_size.checked_add(bytes_needed(stride, chroma_rows, chroma_row_bytes)?)? > src.len() {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    for (row, dst) in out.chunks_exact_mut(w * 4).enumerate() {
        let luma = &src[row * stride..row * stride + w];
        let uv_start = y_size + (row / 2) * stride;
        let chroma = &src[uv_start..uv_start + chroma_row_bytes];
        for ((pair, ys), uv) in dst.chunks_mut(8).zip(luma.chunks(2)).zip(chroma.chunks_exact(2)) {
            yuv_pixel(ys[0], uv[0], uv[1], &mut pair[..4]);
            if ys.len() == 2 {
                yuv_pixel(ys[1], uv[0], uv[1], &mut pair[4..]);
            }
        }
    }
    Some(out)
}

// ---- Motion JPEG -------------------------------------------------------------------

// UVC cameras may leave the Huffman tables out of each frame and expect the
// decoder to use the standard ones from the JPEG specification (ITU-T T.81,
// Annex K.3). Each table is: class and ID, 16 code-length counts, then the values.
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];
const AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14,
    0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09,
    0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65,
    0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9,
    0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA,
    0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA,
    0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32,
    0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16,
    0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64,
    0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86,
    0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8,
    0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9,
    0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// One DHT (define Huffman tables) segment holding all four standard tables.
fn standard_huffman_segment() -> Vec<u8> {
    let tables: [(u8, &[u8; 16], &[u8]); 4] = [
        (0x00, &DC_LUMA_BITS, &DC_VALUES),
        (0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES),
        (0x01, &DC_CHROMA_BITS, &DC_VALUES),
        (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES),
    ];
    let mut body = Vec::new();
    for (class_and_id, bits, values) in tables {
        body.push(class_and_id);
        body.extend_from_slice(bits);
        body.extend_from_slice(values);
    }
    let mut segment = vec![0xFF, 0xC4];
    segment.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    segment.extend(body);
    segment
}

/// Inserts the standard Huffman tables just before the start-of-scan marker. Returns
/// None when the frame isn't a JPEG, is cut short before the scan, or already has
/// tables of its own (then they aren't the problem).
fn with_standard_huffman_tables(src: &[u8]) -> Option<Vec<u8>> {
    const SOI: u8 = 0xD8;
    const SOS: u8 = 0xDA;
    const DHT: u8 = 0xC4;
    if src.len() < 4 || src[0] != 0xFF || src[1] != SOI {
        return None;
    }
    let mut pos = 2;
    let sos = loop {
        if pos + 1 >= src.len() || src[pos] != 0xFF {
            return None;
        }
        match src[pos + 1] {
            0xFF => pos += 1, // fill byte
            SOS => break pos,
            DHT => return None,
            // Markers without a length field.
            0x01 | 0xD0..=0xD7 => pos += 2,
            _ => {
                if pos + 3 >= src.len() {
                    return None;
                }
                let len = u16::from_be_bytes([src[pos + 2], src[pos + 3]]) as usize;
                if len < 2 {
                    return None;
                }
                pos += 2 + len;
            }
        }
    };
    let mut out = Vec::with_capacity(src.len() + 432);
    out.extend_from_slice(&src[..sos]);
    out.extend(standard_huffman_segment());
    out.extend_from_slice(&src[sos..]);
    Some(out)
}

fn decode_jpeg(src: &[u8], max_width: u32, max_height: u32) -> Option<(Vec<u8>, u32, u32)> {
    // Never larger than the size agreed with the driver, so a damaged frame can't
    // make the decoder allocate far more than a real one needs.
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::BGRA)
        .set_max_width(max_width.clamp(1, 16384) as usize)
        .set_max_height(max_height.clamp(1, 16384) as usize);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(src), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (info.width as u32, info.height as u32);
    let ok = safety::geometry(width, height) && pixels.len() == width as usize * height as usize * 4;
    ok.then_some((pixels, width, height))
}

/// Decodes one Motion-JPEG frame of at most `max_width` × `max_height` pixels.
/// Returns the pixels and their width and height.
pub fn mjpeg_to_bgra(src: &[u8], max_width: u32, max_height: u32) -> Option<(Vec<u8>, u32, u32)> {
    decode_jpeg(src, max_width, max_height)
        .or_else(|| decode_jpeg(&with_standard_huffman_tables(src)?, max_width, max_height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bgra_at(px: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    fn close(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
        a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance)
    }

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    // BT.601 limited-range red, green and blue (Y, U, V).
    const RED_YUV: (u8, u8, u8) = (81, 90, 240);
    const GREEN_YUV: (u8, u8, u8) = (145, 54, 34);
    const BLUE_YUV: (u8, u8, u8) = (41, 240, 110);

    #[test]
    fn yuv_known_colours() {
        let mut px = [0u8; 4];
        yuv_pixel(235, 128, 128, &mut px);
        assert_eq!(px, WHITE);
        yuv_pixel(16, 128, 128, &mut px);
        assert_eq!(px, BLACK);
        yuv_pixel(RED_YUV.0, RED_YUV.1, RED_YUV.2, &mut px);
        assert!(close(px, [0, 0, 255, 255], 2), "{px:?}");
        yuv_pixel(GREEN_YUV.0, GREEN_YUV.1, GREEN_YUV.2, &mut px);
        assert!(close(px, [0, 255, 0, 255], 2), "{px:?}");
        yuv_pixel(BLUE_YUV.0, BLUE_YUV.1, BLUE_YUV.2, &mut px);
        assert!(close(px, [255, 0, 0, 255], 2), "{px:?}");
        // Out-of-range input clamps instead of wrapping.
        yuv_pixel(255, 255, 255, &mut px);
        assert_eq!((px[0], px[2], px[3]), (255, 255, 255)); // blue and red saturate
        yuv_pixel(0, 128, 128, &mut px);
        assert_eq!(px, BLACK);
    }

    #[test]
    fn yuyv_converts_pairs() {
        // White then black share chroma.
        let src = [235, 128, 16, 128];
        let out = yuyv_to_bgra(&src, 2, 1, 4).unwrap();
        assert_eq!(out, [WHITE, BLACK].concat());
    }

    #[test]
    fn yuyv_odd_width_and_padding() {
        // 3×2 with an 8-byte row group (2 groups) plus 2 padding bytes per row.
        let (r, w) = (RED_YUV, (235u8, 128u8, 128u8));
        let row0 = [w.0, w.1, 16, w.2, r.0, r.1, 99, r.2, 0xEE, 0xEE];
        let row1 = [r.0, r.1, r.0, r.2, w.0, w.1, 99, w.2];
        let src: Vec<u8> = row0.iter().chain(row1.iter()).copied().collect();
        let out = yuyv_to_bgra(&src, 3, 2, 10).unwrap();
        assert_eq!(out.len(), 3 * 2 * 4);
        assert_eq!(bgra_at(&out, 3, 0, 0), WHITE);
        assert_eq!(bgra_at(&out, 3, 1, 0), BLACK);
        assert!(close(bgra_at(&out, 3, 2, 0), [0, 0, 255, 255], 2));
        assert!(close(bgra_at(&out, 3, 0, 1), [0, 0, 255, 255], 2));
        assert_eq!(bgra_at(&out, 3, 2, 1), WHITE);
    }

    #[test]
    fn yuyv_rejects_bad_sizes() {
        let src = [128u8; 16];
        assert!(yuyv_to_bgra(&src, 2, 2, 4).is_some()); // exactly enough (8 bytes used)
        assert!(yuyv_to_bgra(&src, 2, 5, 4).is_none()); // short buffer
        assert!(yuyv_to_bgra(&src, 4, 1, 4).is_none()); // stride smaller than a row
        assert!(yuyv_to_bgra(&src, 0, 1, 4).is_none());
        assert!(yuyv_to_bgra(&src, 1, 0, 4).is_none());
        assert!(yuyv_to_bgra(&[], 2, 1, 4).is_none());
        assert!(yuyv_to_bgra(&src, 2, 2, usize::MAX).is_none()); // overflow
        assert!(yuyv_to_bgra(&src, 100_000, 1, 400_000).is_none()); // too large
    }

    #[test]
    fn nv12_known_colours() {
        // 2×2: one red block.
        let r = RED_YUV;
        let src = [r.0, r.0, r.0, r.0, r.1, r.2];
        let out = nv12_to_bgra(&src, 2, 2, 2).unwrap();
        for i in 0..4 {
            assert!(close(bgra_at(&out, 2, i % 2, i / 2), [0, 0, 255, 255], 2));
        }
        // Luma varies per pixel while chroma is shared.
        let src = [235, 16, 16, 235, 128, 128];
        let out = nv12_to_bgra(&src, 2, 2, 2).unwrap();
        assert_eq!(out, [WHITE, BLACK, BLACK, WHITE].concat());
    }

    #[test]
    fn nv12_odd_size_and_stride() {
        // 3×3 with stride 4: Y rows of 3 (+1 padding), then 2 UV rows of 2 pairs.
        let b = BLUE_YUV;
        #[rustfmt::skip]
        let src = [
            235, 235, 16, 0,
            235, 235, 16, 0,
            16, 16, 235, 0,
            128, 128, b.1, b.2,
            b.1, b.2, 128, 128,
        ];
        let out = nv12_to_bgra(&src, 3, 3, 4).unwrap();
        assert_eq!(out.len(), 3 * 3 * 4);
        assert_eq!(bgra_at(&out, 3, 0, 0), WHITE);
        assert_eq!(bgra_at(&out, 3, 1, 1), WHITE);
        // (2, 0) uses the second UV pair of row 0: blue chroma on black luma.
        let p = bgra_at(&out, 3, 2, 0);
        assert!(p[0] > p[2] && p[0] > p[1], "{p:?}");
        // (2, 2) uses the second UV pair of row 1: neutral.
        assert_eq!(bgra_at(&out, 3, 2, 2), WHITE);
        // The 20 bytes above are exactly enough; one fewer is not.
        assert!(nv12_to_bgra(&src[..19], 3, 3, 4).is_none());
    }

    #[test]
    fn nv12_rejects_bad_sizes() {
        let src = [128u8; 6];
        assert!(nv12_to_bgra(&src, 2, 2, 2).is_some());
        assert!(nv12_to_bgra(&src[..5], 2, 2, 2).is_none());
        assert!(nv12_to_bgra(&src, 2, 2, 1).is_none());
        assert!(nv12_to_bgra(&src, 3, 1, 3).is_none()); // odd width needs stride 4 for UV
        assert!(nv12_to_bgra(&src, 0, 2, 2).is_none());
        assert!(nv12_to_bgra(&src, 2, 2, usize::MAX).is_none());
        assert!(nv12_to_bgra(&[], 2, 2, 2).is_none());
    }

    // Solid-colour JPEGs made with the standard tables, then stripped of their
    // Huffman tables (DHT) and JFIF header, as UVC cameras send them.
    // 16×16, 4:2:2, decodes to RGB (202, 40, 90).
    #[rustfmt::skip]
    const COLOUR_NO_DHT: [u8; 183] = [
        0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08,
        0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C,
        0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34,
        0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x09, 0x09, 0x09, 0x0C,
        0x0B, 0x0C, 0x18, 0x0D, 0x0D, 0x18, 0x32, 0x21, 0x1C, 0x21, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10, 0x03, 0x01, 0x21, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF,
        0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0xE7, 0x68, 0xA9, 0x3F, 0x44, 0x0A, 0x28,
        0x03, 0xFF, 0xD9,
    ];
    // 17×9 (odd size), 4:2:0, decodes to RGB (30, 160, 220).
    #[rustfmt::skip]
    const ODD_NO_DHT: [u8; 186] = [
        0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08,
        0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C,
        0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34,
        0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x09, 0x09, 0x09, 0x0C,
        0x0B, 0x0C, 0x18, 0x0D, 0x0D, 0x18, 0x32, 0x21, 0x1C, 0x21, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
        0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x09, 0x00, 0x11, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF,
        0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0x28, 0xA2, 0x8A, 0xFA, 0xE3, 0xE0, 0x42,
        0x8A, 0x28, 0xA0, 0x0F, 0xFF, 0xD9,
    ];

    #[test]
    fn standard_tables_are_well_formed() {
        for (bits, count) in [
            (DC_LUMA_BITS, DC_VALUES.len()),
            (DC_CHROMA_BITS, DC_VALUES.len()),
            (AC_LUMA_BITS, AC_LUMA_VALUES.len()),
            (AC_CHROMA_BITS, AC_CHROMA_VALUES.len()),
        ] {
            assert_eq!(bits.iter().map(|&b| b as usize).sum::<usize>(), count);
        }
        let segment = standard_huffman_segment();
        assert_eq!(&segment[..2], &[0xFF, 0xC4]);
        assert_eq!(u16::from_be_bytes([segment[2], segment[3]]) as usize, segment.len() - 2);
    }

    #[test]
    fn mjpeg_without_huffman_tables() {
        // The decoder alone can't read these frames...
        assert!(decode_jpeg(&COLOUR_NO_DHT, 16384, 16384).is_none());
        // ...but with the standard tables inserted it can.
        let (px, w, h) = mjpeg_to_bgra(&COLOUR_NO_DHT, 16384, 16384).unwrap();
        assert_eq!((w, h), (16, 16));
        assert_eq!(px.len(), 16 * 16 * 4);
        for (x, y) in [(0, 0), (15, 15), (7, 9)] {
            let p = bgra_at(&px, w, x, y);
            assert!(close(p, [90, 40, 202, 255], 4), "{p:?} at {x},{y}");
        }
    }

    #[test]
    fn mjpeg_odd_size() {
        let (px, w, h) = mjpeg_to_bgra(&ODD_NO_DHT, 16384, 16384).unwrap();
        assert_eq!((w, h), (17, 9));
        assert_eq!(px.len(), 17 * 9 * 4);
        for (x, y) in [(0, 0), (16, 8), (16, 0)] {
            let p = bgra_at(&px, w, x, y);
            assert!(close(p, [220, 160, 30, 255], 4), "{p:?} at {x},{y}");
        }
    }

    #[test]
    fn mjpeg_with_tables_of_its_own() {
        let full = with_standard_huffman_tables(&COLOUR_NO_DHT).unwrap();
        assert!(decode_jpeg(&full, 16384, 16384).is_some());
        // Tables already present: nothing to insert.
        assert!(with_standard_huffman_tables(&full).is_none());
        assert!(mjpeg_to_bgra(&full, 16384, 16384).is_some());
    }

    #[test]
    fn mjpeg_bad_frames_are_skipped() {
        assert!(mjpeg_to_bgra(&[], 16384, 16384).is_none());
        // A frame larger than the size agreed with the driver is refused.
        assert!(mjpeg_to_bgra(&COLOUR_NO_DHT, 8, 8).is_none());
        assert!(mjpeg_to_bgra(&[0xFF, 0xD8], 16384, 16384).is_none());
        assert!(mjpeg_to_bgra(b"not a jpeg at all", 16384, 16384).is_none());
        // Cut off at every length: never panics. Cut before the scan: no image.
        for n in 0..COLOUR_NO_DHT.len() {
            let result = mjpeg_to_bgra(&COLOUR_NO_DHT[..n], 16384, 16384);
            if n < 160 {
                assert!(result.is_none(), "cut at {n}");
            }
        }
        // Random damage anywhere in the frame: skipped or decoded, never a panic
        // (release builds abort on panic, which would close the app).
        let mut seed: u32 = 12345;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) as usize
        };
        for _ in 0..3000 {
            let mut frame = COLOUR_NO_DHT.to_vec();
            for _ in 0..1 + next() % 4 {
                let i = next() % frame.len();
                frame[i] = next() as u8;
            }
            let cut = frame.len() - next() % 8;
            let _ = mjpeg_to_bgra(&frame[..cut], 16384, 16384);
        }
        // Damaged segment length.
        let mut bad = COLOUR_NO_DHT;
        bad[4] = 0xFF;
        assert!(mjpeg_to_bgra(&bad, 16384, 16384).is_none());
    }
}
