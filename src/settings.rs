//! Settings: a small key=value text file in the user's settings folder
//! (%APPDATA%\obsbot-control on Windows, ~/.config/obsbot-control on Linux).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preset {
    pub saved: bool,
    pub pan: i32,
    pub tilt: i32,
    pub zoom: i32,
}

/// Hold-to-move methods (Settings::move_method).
pub const MOVE_GIMBAL_SPEED: i32 = 0;
pub const MOVE_UVC_STEPS: i32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub preview_enabled: bool,
    /// -1 leave the camera alone, 0 off, 1 = 50 Hz (NZ mains), 2 = 60 Hz.
    pub anti_flicker: i32,
    /// 0 automatic, 1 Tiny 2 codes, 2 Lite codes.
    pub variant: i32,
    pub move_method: i32,
    /// Degrees per second for hold-to-move.
    pub move_speed: f32,
    pub invert_horizontal: bool,
    pub invert_vertical: bool,
    pub show_developer: bool,
    pub auto_compact: bool,
    pub pause_in_background: bool,
    pub presets: [Preset; 3],
    /// The last camera used (DeviceInfo::path).
    pub device_path: String,
    pub window_w: i32,
    pub window_h: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            preview_enabled: true,
            anti_flicker: 1,
            variant: 0,
            move_method: MOVE_GIMBAL_SPEED,
            move_speed: 30.0,
            invert_horizontal: false,
            invert_vertical: false,
            show_developer: false,
            auto_compact: true,
            pause_in_background: true,
            presets: [Preset::default(); 3],
            device_path: String::new(),
            window_w: 1280,
            window_h: 800,
        }
    }
}

/// Parses a whole number the way std::from_chars does: no spaces, no '+', nothing
/// left over. Anything else gives the fallback.
fn parse_int(text: Option<&String>, fallback: i32) -> i32 {
    match text {
        Some(t) if !t.starts_with('+') => t.parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn parse_float(text: Option<&String>, fallback: f32) -> f32 {
    let Some(t) = text else { return fallback };
    if t.starts_with('+') {
        return fallback;
    }
    match t.parse::<f32>() {
        Ok(v) if v.is_finite() => {
            // Rust rounds a too-small number to zero; from_chars calls that out of range.
            let mantissa = t.split(['e', 'E']).next().unwrap_or("");
            if v == 0.0 && mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b)) { fallback } else { v }
        }
        _ => fallback,
    }
}

/// A switch is 0 or 1; any other number keeps the default.
fn parse_bool(text: Option<&String>, fallback: bool) -> bool {
    match parse_int(text, fallback as i32) {
        0 => false,
        1 => true,
        _ => fallback,
    }
}

const FILE_NAME: &str = "settings.ini";

impl Settings {
    /// Reads the settings file; defaults when it is missing or damaged.
    pub fn load() -> Settings {
        match Settings::directory().and_then(|d| std::fs::read(d.join(FILE_NAME)).ok()) {
            // Lossy, so a stray bad byte cannot throw away every other setting.
            Some(bytes) => Settings::parse(&String::from_utf8_lossy(&bytes)),
            None => Settings::default(),
        }
    }

    /// Writes the file safely (a temporary file, then a rename).
    pub fn save(&self) {
        if let Some(dir) = Settings::directory() {
            write_atomic(&dir, &self.to_text());
        }
    }

    /// Parses the file's text. Out-of-range values fall back to the defaults.
    pub fn parse(text: &str) -> Settings {
        let mut kv: HashMap<&str, String> = HashMap::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                kv.insert(k, v.to_string());
            }
        }
        let get = |k: &str| kv.get(k);

        let d = Settings::default();
        let mut s = d.clone();
        s.preview_enabled = parse_bool(get("preview"), s.preview_enabled);
        s.anti_flicker = parse_int(get("anti_flicker"), s.anti_flicker);
        s.variant = parse_int(get("protocol_variant"), s.variant);
        s.move_method = parse_int(get("move_method"), s.move_method);
        s.move_speed = parse_float(get("move_speed"), s.move_speed);
        // The older invert_pan/invert_tilt keys are deliberately not read: the default
        // directions changed with the rename, so carrying them over could flip an axis twice.
        s.invert_horizontal = parse_bool(get("invert_horizontal"), s.invert_horizontal);
        s.invert_vertical = parse_bool(get("invert_vertical"), s.invert_vertical);
        s.show_developer = parse_bool(get("show_developer"), s.show_developer);
        s.auto_compact = parse_bool(get("auto_compact"), s.auto_compact);
        s.pause_in_background = parse_bool(get("pause_in_background"), s.pause_in_background);
        // A hand-edited or damaged file must not produce out-of-range values (the
        // move speed goes straight to the gimbal). Fall back to the defaults.
        if !(-1..=2).contains(&s.anti_flicker) {
            s.anti_flicker = d.anti_flicker;
        }
        if !(0..=2).contains(&s.variant) {
            s.variant = d.variant;
        }
        if !(0..=1).contains(&s.move_method) {
            s.move_method = d.move_method;
        }
        if !s.move_speed.is_finite() {
            s.move_speed = d.move_speed;
        }
        s.move_speed = s.move_speed.clamp(5.0, 90.0); // same range as the speed slider
        s.window_w = parse_int(get("window_w"), s.window_w);
        s.window_h = parse_int(get("window_h"), s.window_h);
        if let Some(path) = get("device_path") {
            s.device_path = path.clone();
        }
        for (i, preset) in s.presets.iter_mut().enumerate() {
            let key = |field: &str| format!("preset{}_{field}", i + 1);
            preset.saved = parse_int(get(&key("saved")), 0) == 1;
            preset.pan = parse_int(get(&key("pan")), 0);
            preset.tilt = parse_int(get(&key("tilt")), 0);
            preset.zoom = parse_int(get(&key("zoom")), 0);
        }
        s
    }

    /// The file's text (LF line endings).
    pub fn to_text(&self) -> String {
        let b = |v: bool| v as i32;
        let mut t = String::new();
        t.push_str("# obsbot-control settings. Safe to delete; defaults will be used.\n");
        t.push_str(&format!("preview={}\n", b(self.preview_enabled)));
        t.push_str(&format!("anti_flicker={}\n", self.anti_flicker));
        t.push_str(&format!("protocol_variant={}\n", self.variant));
        t.push_str(&format!("move_method={}\n", self.move_method));
        t.push_str(&format!("move_speed={:.1}\n", self.move_speed));
        t.push_str(&format!("invert_horizontal={}\n", b(self.invert_horizontal)));
        t.push_str(&format!("invert_vertical={}\n", b(self.invert_vertical)));
        t.push_str(&format!("show_developer={}\n", b(self.show_developer)));
        t.push_str(&format!("auto_compact={}\n", b(self.auto_compact)));
        t.push_str(&format!("pause_in_background={}\n", b(self.pause_in_background)));
        t.push_str(&format!("window_w={}\n", self.window_w));
        t.push_str(&format!("window_h={}\n", self.window_h));
        t.push_str(&format!("device_path={}\n", self.device_path));
        for (i, p) in self.presets.iter().enumerate() {
            let n = i + 1;
            t.push_str(&format!("preset{n}_saved={}\n", b(p.saved)));
            t.push_str(&format!("preset{n}_pan={}\n", p.pan));
            t.push_str(&format!("preset{n}_tilt={}\n", p.tilt));
            t.push_str(&format!("preset{n}_zoom={}\n", p.zoom));
        }
        t
    }

    /// The settings folder, created on demand. None disables saving.
    pub fn directory() -> Option<PathBuf> {
        static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
        DIRECTORY.get_or_init(find_directory).clone()
    }
}

/// Only replaces the old file if the new one was written completely (e.g. not on a full disk).
fn write_atomic(dir: &Path, text: &str) {
    use std::io::Write;
    let path = dir.join(FILE_NAME);
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    let written = std::fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(text.as_bytes())?;
        f.sync_all()
    });
    if written.is_err() || std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn find_directory() -> Option<PathBuf> {
    let dir = base_directory()?;
    create_private(&dir)?;
    if !dir.is_dir() {
        return None;
    }
    #[cfg(windows)]
    migrate_old_settings(&dir);
    Some(dir)
}

#[cfg(windows)]
fn base_directory() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(appdata).join("obsbot-control"))
}

#[cfg(not(windows))]
fn base_directory() -> Option<PathBuf> {
    config_home(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME")).map(|d| d.join("obsbot-control"))
}

/// $XDG_CONFIG_HOME if it is an absolute path (the spec says to ignore relative
/// ones), else $HOME/.config.
#[cfg(not(windows))]
fn config_home(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if let Some(x) = xdg {
        let p = PathBuf::from(x);
        if p.is_absolute() {
            return Some(p);
        }
    }
    let home = home.filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".config"))
}

/// Creates the folder (private to the user on Unix). An existing one is fine.
fn create_private(dir: &Path) -> Option<()> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    #[cfg(unix)]
    let made = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(dir)
    };
    #[cfg(not(unix))]
    let made = std::fs::create_dir(dir);
    match made {
        Ok(()) => Some(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Some(()),
        Err(_) => None,
    }
}

/// The app used to be called osbot-oss-windows; bring its file across once.
#[cfg(windows)]
fn migrate_old_settings(dir: &Path) {
    let new_file = dir.join(FILE_NAME);
    if new_file.exists() {
        return;
    }
    if let Some(parent) = dir.parent() {
        let old = parent.join("osbot-oss-windows").join(FILE_NAME);
        if old.is_file() {
            let _ = std::fs::copy(old, new_file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        Settings {
            preview_enabled: false,
            anti_flicker: 2,
            variant: 2,
            move_method: MOVE_UVC_STEPS,
            move_speed: 45.5,
            invert_horizontal: true,
            invert_vertical: true,
            show_developer: true,
            auto_compact: false,
            pause_in_background: false,
            presets: [
                Preset { saved: true, pan: -1234, tilt: 567, zoom: 3 },
                Preset::default(),
                Preset { saved: true, pan: 1, tilt: -2, zoom: 100 },
            ],
            device_path: "\\\\?\\usb#vid_3564&pid_fef9#caf\u{e9}".to_string(),
            window_w: 900,
            window_h: 600,
        }
    }

    #[test]
    fn round_trip() {
        let s = sample();
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert_eq!(Settings::parse(&Settings::default().to_text()), Settings::default());
    }

    #[test]
    fn text_format() {
        let t = Settings::default().to_text();
        assert!(t.starts_with("# obsbot-control settings. Safe to delete; defaults will be used.\n"));
        assert!(!t.contains('\r'));
        assert!(t.contains("move_speed=30.0\n"));
        assert!(t.contains("preset3_zoom=0\n"));
        assert!(t.ends_with('\n'));
    }

    #[test]
    fn empty_and_junk_give_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::parse("\n\n   \nnot a setting\n=\n;preview=0\n#preview=0\n"), Settings::default());
    }

    #[test]
    fn crlf_files_load() {
        let text = sample().to_text().replace('\n', "\r\n");
        assert_eq!(Settings::parse(&text), sample());
    }

    #[test]
    fn out_of_range_values_fall_back() {
        let s = Settings::parse("anti_flicker=3\nprotocol_variant=-1\nmove_method=2\npreview=2\nauto_compact=-1\n");
        let d = Settings::default();
        assert_eq!(s.anti_flicker, d.anti_flicker);
        assert_eq!(s.variant, d.variant);
        assert_eq!(s.move_method, d.move_method);
        assert_eq!(s.preview_enabled, d.preview_enabled);
        assert_eq!(s.auto_compact, d.auto_compact);
        assert_eq!(Settings::parse("anti_flicker=-1\n").anti_flicker, -1);
        assert_eq!(Settings::parse("anti_flicker=0\n").anti_flicker, 0);
    }

    #[test]
    fn move_speed_is_clamped_or_defaulted() {
        assert_eq!(Settings::parse("move_speed=1000\n").move_speed, 90.0);
        assert_eq!(Settings::parse("move_speed=-3\n").move_speed, 5.0);
        assert_eq!(Settings::parse("move_speed=25.5\n").move_speed, 25.5);
        for bad in ["nan", "inf", "-inf", "1e1000", "1e-1000", "20junk", "", "+20", " 20"] {
            assert_eq!(Settings::parse(&format!("move_speed={bad}\n")).move_speed, 30.0, "{bad:?}");
        }
    }

    #[test]
    fn numbers_must_be_whole_and_plain() {
        for bad in ["12x", "1.5", " 1", "1 ", "+1", "99999999999", ""] {
            assert_eq!(Settings::parse(&format!("window_w={bad}\n")).window_w, 1280, "{bad:?}");
        }
        assert_eq!(Settings::parse("window_w=800\nwindow_h=-5\n").window_h, -5);
    }

    #[test]
    fn old_invert_keys_are_ignored() {
        let s = Settings::parse("invert_pan=1\ninvert_tilt=1\n");
        assert!(!s.invert_horizontal && !s.invert_vertical);
        let s = Settings::parse("invert_horizontal=1\ninvert_vertical=1\n");
        assert!(s.invert_horizontal && s.invert_vertical);
    }

    #[test]
    fn presets_load() {
        let s = Settings::parse("preset2_saved=1\npreset2_pan=-40\npreset2_tilt=x\npreset3_saved=2\npreset3_pan=9\n");
        assert_eq!(s.presets[0], Preset::default());
        assert_eq!(s.presets[1], Preset { saved: true, pan: -40, tilt: 0, zoom: 0 });
        assert!(!s.presets[2].saved);
        assert_eq!(s.presets[2].pan, 9);
    }

    #[test]
    fn device_path_keeps_equals_signs_and_last_value_wins() {
        let s = Settings::parse("device_path=a=b\ndevice_path=c=d=e\n");
        assert_eq!(s.device_path, "c=d=e");
    }

    #[cfg(not(windows))]
    #[test]
    fn config_home_rules() {
        use std::ffi::OsString;
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(config_home(os("/x/cfg"), os("/home/u")), Some(PathBuf::from("/x/cfg")));
        assert_eq!(config_home(os("relative"), os("/home/u")), Some(PathBuf::from("/home/u/.config")));
        assert_eq!(config_home(os(""), os("/home/u")), Some(PathBuf::from("/home/u/.config")));
        assert_eq!(config_home(None, os("/home/u")), Some(PathBuf::from("/home/u/.config")));
        assert_eq!(config_home(None, None), None);
        assert_eq!(config_home(None, os("")), None);
    }

    #[test]
    fn write_atomic_replaces_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("obsbot-control-test-{}", std::process::id()));
        create_private(&dir).unwrap();
        write_atomic(&dir, "first\n");
        write_atomic(&dir, "second\n");
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), "second\n");
        assert!(!dir.join("settings.ini.tmp").exists());
        // A failed write (the target is a directory) must leave no temp file behind.
        std::fs::remove_file(dir.join(FILE_NAME)).unwrap();
        std::fs::create_dir(dir.join(FILE_NAME)).unwrap();
        write_atomic(&dir, "third\n");
        assert!(!dir.join("settings.ini.tmp").exists());
        assert!(dir.join(FILE_NAME).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
