//! When the window switches itself between full and compact view, and how Windows'
//! camera-use records are read. Portable so it can be tested anywhere.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    None,
    ToCompact,
    ToFull,
}

/// Reacts only when another app starts or stops using the camera, so a choice made
/// with the Compact button stays until the next change.
#[derive(Clone, Copy, Debug, Default)]
pub struct Policy {
    in_use: bool,
    automatic: bool,
}

impl Policy {
    pub fn on_detected(&mut self, in_use: bool, compact_now: bool) -> Switch {
        if in_use == self.in_use {
            return Switch::None;
        }
        self.in_use = in_use;
        if in_use {
            if compact_now {
                return Switch::None; // already compact by choice: keep it after the call
            }
            self.automatic = true;
            return Switch::ToCompact;
        }
        let was_automatic = self.automatic;
        self.automatic = false;
        if was_automatic && compact_now { Switch::ToFull } else { Switch::None }
    }
    pub fn on_manual_toggle(&mut self) {
        self.automatic = false;
    }
}

/// Windows sets LastUsedTimeStart when an app opens a camera and LastUsedTimeStop
/// (0 until then) when it lets go.
pub fn in_use(start: u64, stop: u64) -> bool {
    start != 0 && start > stop
}

/// A desktop app's registry key is its exe path with '\' written as '#'.
pub fn is_same_path(key: &str, exe_path: &str) -> bool {
    let mut a = key.chars();
    let mut b = exe_path.chars().map(|c| if c == '\\' { '#' } else { c });
    if key.is_empty() {
        return false;
    }
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if lower(x) == lower(y) => {}
            _ => return false,
        }
    }
}

// Simple one-to-one lower casing, like towlower: a char that lowercases to several
// chars is left as it is.
fn lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// "C:#Program Files#Zoom#bin#Zoom.exe" -> "Zoom.exe"
pub fn exe_file_name(key: &str) -> String {
    key.rsplit('#').next().unwrap_or(key).to_string()
}

/// A readable name: "C:#...#Zoom.exe" -> "Zoom"; "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "WindowsCamera".
pub fn app_name(key: &str, packaged: bool) -> String {
    let mut name = key.to_string();
    if packaged {
        if let Some(i) = name.find('_') {
            name.truncate(i);
        }
        if let Some(dot) = name.rfind('.') {
            if dot + 1 < name.len() {
                name.drain(..=dot);
            }
        }
    } else {
        name = exe_file_name(&name);
        // Compare bytes so a multi-byte character near the end cannot split.
        let bytes = name.as_bytes();
        if bytes.len() > 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".exe") {
            name.truncate(name.len() - 4);
        }
    }
    if name.is_empty() { key.to_string() } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_starts_and_stops() {
        let mut p = Policy::default();
        assert_eq!(p.on_detected(false, false), Switch::None);
        assert_eq!(p.on_detected(true, false), Switch::ToCompact);
        assert_eq!(p.on_detected(true, true), Switch::None); // only changes count
        assert_eq!(p.on_detected(false, true), Switch::ToFull);
        assert_eq!(p.on_detected(false, false), Switch::None);
    }

    #[test]
    fn opened_during_a_call() {
        let mut p = Policy::default();
        assert_eq!(p.on_detected(true, false), Switch::ToCompact);
    }

    #[test]
    fn back_to_full_by_hand_stays_full() {
        let mut p = Policy::default();
        assert_eq!(p.on_detected(true, false), Switch::ToCompact);
        p.on_manual_toggle();
        assert_eq!(p.on_detected(true, false), Switch::None);
        assert_eq!(p.on_detected(false, false), Switch::None);
        // The next call switches again.
        assert_eq!(p.on_detected(true, false), Switch::ToCompact);
    }

    #[test]
    fn compact_by_hand_stays_compact() {
        let mut p = Policy::default();
        p.on_manual_toggle();
        assert_eq!(p.on_detected(true, true), Switch::None);
        assert_eq!(p.on_detected(false, true), Switch::None);
    }

    #[test]
    fn toggled_twice_is_the_users_choice() {
        let mut p = Policy::default();
        assert_eq!(p.on_detected(true, false), Switch::ToCompact);
        p.on_manual_toggle();
        p.on_manual_toggle();
        assert_eq!(p.on_detected(false, true), Switch::None);
    }

    #[test]
    fn camera_use_records() {
        assert!(in_use(200, 0));
        assert!(in_use(200, 100));
        assert!(!in_use(100, 200));
        assert!(!in_use(0, 0));
    }

    #[test]
    fn same_path() {
        assert!(is_same_path("C:#Tools#obsbot-control.exe", "c:\\tools\\OBSBOT-CONTROL.EXE"));
        assert!(!is_same_path("C:#Tools#other.exe", "C:\\Tools\\obsbot.exe"));
        assert!(!is_same_path("", ""));
        assert!(!is_same_path("C:#Tools", "C:\\Tools\\x"));
    }

    #[test]
    fn names() {
        assert_eq!(app_name("C:#Program Files#Zoom#bin#Zoom.exe", false), "Zoom");
        assert_eq!(app_name("C:#Users#me#AppData#Local#Discord#app-1.0.9#Discord.EXE", false), "Discord");
        assert_eq!(app_name("C:#Tools#obs64", false), "obs64");
        assert_eq!(app_name("Microsoft.WindowsCamera_8wekyb3d8bbwe", true), "WindowsCamera");
        assert_eq!(app_name("MSTeams_8wekyb3d8bbwe", true), "MSTeams");
        assert_eq!(app_name("C:#", false), "C:#");
        assert_eq!(app_name("caf\u{e9}\u{e9}", false), "caf\u{e9}\u{e9}");
        assert_eq!(exe_file_name("C:#Program Files#Zoom#bin#Zoom.exe"), "Zoom.exe");
        assert_eq!(exe_file_name("Zoom.exe"), "Zoom.exe");
        assert_eq!(exe_file_name("C:#"), "");
    }
}
