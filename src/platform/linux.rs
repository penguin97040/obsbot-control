//! Linux services: single instance lock, window styling and the system version.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use super::InstanceGuard;

/// Where the lock file lives: the per-user runtime folder when there is one
/// (cleared at logout), otherwise a per-user name in /tmp.
fn lock_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        if dir.is_absolute() && dir.is_dir() {
            return dir.join("obsbot-control.lock");
        }
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/obsbot-control-{uid}.lock"))
}

/// Takes an exclusive lock on `path` without waiting. None when another process
/// holds it. The lock lasts as long as the returned file stays open, and the
/// kernel drops it if the app crashes.
fn try_lock(path: &std::path::Path) -> std::io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        // Never follow a symlink someone else planted in /tmp.
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    loop {
        // SAFETY: flock on a descriptor we own, which stays open for the call.
        let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if r == 0 {
            return Ok(Some(file));
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EWOULDBLOCK) => return Ok(None),
            _ => return Err(err),
        }
    }
}

/// None when another copy of the app is already running.
pub fn single_instance() -> Option<InstanceGuard> {
    let path = lock_path();
    match try_lock(&path) {
        Ok(Some(file)) => Some(InstanceGuard::new(file)),
        Ok(None) => None,
        Err(e) => {
            // Better to allow a second copy than to refuse to start at all.
            crate::log_warn!("Could not create the single-instance lock {}: {e}", path.display());
            Some(InstanceGuard::new(()))
        }
    }
}

/// The value of PRETTY_NAME in os-release text, without its quotes.
fn pretty_name(os_release: &str) -> Option<String> {
    let value = os_release.lines().find_map(|line| line.trim().strip_prefix("PRETTY_NAME="))?;
    let value = value.trim();
    let unquoted = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    // Shell-style escapes (\" \\ \$ \`) are allowed in os-release.
    let mut out = String::new();
    let mut chars = unquoted.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// e.g. "Fedora Linux 41 (Workstation Edition), kernel 6.11.4"
pub fn os_version() -> String {
    let name = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().and_then(|text| pretty_name(&text)))
        .unwrap_or_else(|| "Linux".to_string());
    // The same text `uname -r` prints.
    match std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(release) if !release.trim().is_empty() => format!("{name}, kernel {}", release.trim()),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pretty_name() {
        let text = "NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 41 (Workstation Edition)\"\nID=fedora\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Fedora Linux 41 (Workstation Edition)"));
        assert_eq!(pretty_name("PRETTY_NAME='Arch Linux'").as_deref(), Some("Arch Linux"));
        assert_eq!(pretty_name("PRETTY_NAME=Gentoo").as_deref(), Some("Gentoo"));
        assert_eq!(pretty_name("PRETTY_NAME=\"Say \\\"hi\\\"\"").as_deref(), Some("Say \"hi\""));
        assert_eq!(pretty_name("NAME=x\nPRETTY_NAME=\"\""), None);
        assert_eq!(pretty_name(""), None);
    }

    #[test]
    fn os_version_names_the_kernel() {
        assert!(os_version().contains(", kernel "));
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = std::env::temp_dir().join(format!("obsbot-control-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lock");
        let first = try_lock(&path).unwrap();
        assert!(first.is_some());
        // flock locks belong to the open file, so a second open is refused even
        // within one process, as a second copy of the app would be.
        assert!(try_lock(&path).unwrap().is_none());
        drop(first);
        assert!(try_lock(&path).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_a_symlinked_lock() {
        let dir = std::env::temp_dir().join(format!("obsbot-control-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("planted.lock");
        let _ = std::os::unix::fs::symlink(dir.join("target"), &link);
        assert!(try_lock(&link).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
