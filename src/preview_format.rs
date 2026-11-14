//! One of the camera's native capture formats, and the logic behind the preview's
//! resolution, pixel format and frame rate pickers.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewFormat {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// e.g. "NV12", "MJPG", "YUYV"
    pub fourcc: String,
}

impl PreviewFormat {
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 { 0.0 } else { self.fps_num as f64 / self.fps_den as f64 }
    }
    pub fn rounded_fps(&self) -> i32 {
        self.fps().round() as i32
    }
    /// Exact identity, used to ask for this format again: "1280x720:MJPG:30/1".
    pub fn key(&self) -> String {
        format!("{}x{}:{}:{}/{}", self.width, self.height, self.fourcc, self.fps_num, self.fps_den)
    }
    /// e.g. "1280×720 NV12 30 fps"
    pub fn label(&self) -> String {
        format!("{} {} {}", Self::size_label(self.width, self.height), self.fourcc, Self::fps_label(self.rounded_fps()))
    }
    pub fn size_label(width: u32, height: u32) -> String {
        format!("{width}\u{00D7}{height}")
    }
    pub fn fps_label(fps: i32) -> String {
        format!("{fps} fps")
    }
}

pub fn find<'a>(list: &'a [PreviewFormat], key: &str) -> Option<&'a PreviewFormat> {
    list.iter().find(|f| f.key() == key)
}

/// Every frame size offered, largest first.
pub fn resolutions(list: &[PreviewFormat]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for f in list {
        if !out.contains(&(f.width, f.height)) {
            out.push((f.width, f.height));
        }
    }
    // Stable, so equal sizes keep the order the camera listed them in.
    out.sort_by(|a, b| {
        let pa = a.0 as u64 * a.1 as u64;
        let pb = b.0 as u64 * b.1 as u64;
        pb.cmp(&pa).then(b.0.cmp(&a.0))
    });
    out
}

/// The pixel formats offered at one size, in alphabetical order.
pub fn pixel_formats(list: &[PreviewFormat], width: u32, height: u32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in list {
        if f.width == width && f.height == height && !out.contains(&f.fourcc) {
            out.push(f.fourcc.clone());
        }
    }
    out.sort();
    out
}

/// The whole-number frame rates offered for one size and pixel format, fastest first.
pub fn frame_rates(list: &[PreviewFormat], width: u32, height: u32, fourcc: &str) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for f in list {
        if f.width == width && f.height == height && f.fourcc == fourcc && !out.contains(&f.rounded_fps()) {
            out.push(f.rounded_fps());
        }
    }
    out.sort_by(|a, b| b.cmp(a));
    out
}

/// The key of the closest format at width×height (empty when none): the wanted
/// pixel format if it is offered there (else the fallback, else the first), then
/// the nearest frame rate, with ties going to the faster one.
pub fn choose(list: &[PreviewFormat], width: u32, height: u32, fourcc: &str, fps: i32, fallback_fourcc: &str) -> String {
    let pixels = pixel_formats(list, width, height);
    let Some(first) = pixels.first() else {
        return String::new();
    };
    let offered = |p: &str| pixels.iter().any(|x| x == p);
    let pixel = if offered(fourcc) {
        fourcc
    } else if offered(fallback_fourcc) {
        fallback_fourcc
    } else {
        first.as_str()
    };

    let mut best: Option<&PreviewFormat> = None;
    for f in list {
        if f.width != width || f.height != height || f.fourcc != pixel {
            continue;
        }
        match best {
            None => best = Some(f),
            Some(b) => {
                let d = (f.rounded_fps() - fps).abs();
                let bd = (b.rounded_fps() - fps).abs();
                if d < bd || (d == bd && f.fps() > b.fps()) {
                    best = Some(f);
                }
            }
        }
    }
    best.map(|b| b.key()).unwrap_or_default()
}

/// How well a format suits a preview: about 1280×720 at 30 fps scores highest.
pub fn score(format: &PreviewFormat) -> f64 {
    let fps = format.fps();
    let mut score = -((format.width as f64 * format.height as f64) - 1280.0 * 720.0).abs() / 1e5;
    if fps < 29.0 {
        score -= 100.0;
    }
    if fps > 31.0 {
        score -= 1.0; // 30 fps is plenty for a preview
    }
    score
}

/// The best-scoring format (the first one wins a tie), or None for an empty list.
pub fn best_default(list: &[PreviewFormat]) -> Option<&PreviewFormat> {
    let mut best: Option<(&PreviewFormat, f64)> = None;
    for f in list {
        let s = score(f);
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((f, s));
        }
    }
    best.map(|(f, _)| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(w: u32, h: u32, fourcc: &str, num: u32, den: u32) -> PreviewFormat {
        PreviewFormat { width: w, height: h, fps_num: num, fps_den: den, fourcc: fourcc.to_string() }
    }

    fn list() -> Vec<PreviewFormat> {
        vec![
            f(1280, 720, "NV12", 30, 1),
            f(1280, 720, "NV12", 30000, 1001),
            f(1280, 720, "MJPG", 60, 1),
            f(1280, 720, "MJPG", 30, 1),
            f(3840, 2160, "MJPG", 30, 1),
            f(1920, 1080, "NV12", 30, 1),
            f(1920, 1080, "MJPG", 60, 1),
            f(1920, 1080, "YUY2", 5, 1),
            f(640, 360, "YUY2", 30, 1),
            f(1280, 720, "YUY2", 10, 1),
        ]
    }

    #[test]
    fn labels() {
        assert_eq!(PreviewFormat::size_label(1280, 720), "1280\u{00D7}720");
        assert_eq!(f(1280, 720, "NV12", 30000, 1001).label(), "1280\u{00D7}720 NV12 30 fps");
        assert_eq!(f(1, 1, "NV12", 1, 0).fps(), 0.0);
    }

    #[test]
    fn sizes_are_unique_largest_first() {
        let sizes = resolutions(&list());
        assert_eq!(sizes.len(), 4);
        assert_eq!(sizes[0], (3840, 2160));
        assert_eq!(sizes[1], (1920, 1080));
        assert_eq!(sizes[2], (1280, 720));
        assert_eq!(sizes[3], (640, 360));
        assert!(resolutions(&[]).is_empty());
    }

    #[test]
    fn pixel_formats_are_sorted() {
        assert_eq!(pixel_formats(&list(), 1280, 720), vec!["MJPG", "NV12", "YUY2"]);
        assert_eq!(pixel_formats(&list(), 3840, 2160), vec!["MJPG"]);
        assert!(pixel_formats(&list(), 800, 600).is_empty());
    }

    #[test]
    fn frame_rates_show_once_fastest_first() {
        assert_eq!(frame_rates(&list(), 1280, 720, "NV12"), vec![30]);
        assert_eq!(frame_rates(&list(), 1280, 720, "MJPG"), vec![60, 30]);
        assert!(frame_rates(&list(), 1280, 720, "H264").is_empty());
    }

    #[test]
    fn choose_closest() {
        let l = list();
        assert_eq!(choose(&l, 1280, 720, "NV12", 30, ""), "1280x720:NV12:30/1");
        assert_eq!(choose(&l, 1280, 720, "MJPG", 60, ""), "1280x720:MJPG:60/1");
        assert_eq!(choose(&l, 1920, 1080, "NV12", 30, ""), "1920x1080:NV12:30/1");
        assert_eq!(choose(&l, 1920, 1080, "MJPG", 60, ""), "1920x1080:MJPG:60/1");
        // Falls back to the default's format, then the first one offered.
        assert_eq!(choose(&l, 3840, 2160, "NV12", 30, "NV12"), "3840x2160:MJPG:30/1");
        assert_eq!(choose(&l, 640, 360, "NV12", 30, "MJPG"), "640x360:YUY2:30/1");
        assert_eq!(choose(&l, 1920, 1080, "YUY2", 30, ""), "1920x1080:YUY2:5/1");
        assert_eq!(choose(&l, 1920, 1080, "H264", 30, "NV12"), "1920x1080:NV12:30/1");
        // Nearest frame rate; ties go to the faster one.
        assert_eq!(choose(&l, 1280, 720, "MJPG", 50, ""), "1280x720:MJPG:60/1");
        assert_eq!(choose(&l, 1280, 720, "MJPG", 45, ""), "1280x720:MJPG:60/1");
        assert_eq!(choose(&l, 1280, 720, "MJPG", 10, ""), "1280x720:MJPG:30/1");
        // Nothing at that size.
        assert!(choose(&l, 800, 600, "NV12", 30, "").is_empty());
        assert!(choose(&[], 1280, 720, "NV12", 30, "").is_empty());
    }

    #[test]
    fn find_by_key() {
        let l = list();
        assert!(std::ptr::eq(find(&l, "1920x1080:MJPG:60/1").unwrap(), &l[6]));
        assert!(find(&l, "nope").is_none());
    }

    #[test]
    fn scoring() {
        let best = score(&f(1280, 720, "NV12", 30, 1));
        assert_eq!(best, 0.0);
        assert!(score(&f(1280, 720, "MJPG", 60, 1)) < best);
        assert_eq!(score(&f(1280, 720, "MJPG", 60, 1)), -1.0);
        assert!(score(&f(1280, 720, "YUY2", 10, 1)) <= -100.0);
        assert!(score(&f(1920, 1080, "NV12", 30, 1)) < best);
    }

    #[test]
    fn best_default_prefers_720p_30() {
        let l = list();
        // The 30000/1001 NV12 entry is 29.97 fps, so it ties the 30 fps one; the first wins.
        assert!(std::ptr::eq(best_default(&l).unwrap(), &l[0]));
        assert!(best_default(&[]).is_none());
        // With only slow modes, the closest size still wins.
        let slow = vec![f(1920, 1080, "YUY2", 5, 1), f(1280, 720, "YUY2", 10, 1)];
        assert_eq!(best_default(&slow).unwrap().key(), "1280x720:YUY2:10/1");
    }
}
