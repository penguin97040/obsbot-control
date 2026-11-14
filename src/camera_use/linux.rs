//! Linux: is another process streaming from our camera?
//!
//! Only one file handle at a time may own a V4L2 capture stream, so asking for a
//! buffer is a quick, harmless test: the driver says EBUSY when someone else owns
//! it. Only then do we look through /proc for a name to show.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::sync::Mutex;

use super::CameraUse;
use crate::v4l2;

pub fn query(device_path: &str, our_preview_running: bool) -> CameraUse {
    // The name found when another app took the camera, kept while it has it: scanning
    // every process's open files can take a while on a busy desktop, and this runs on
    // the interface thread every 2 seconds.
    static LAST: Mutex<Option<(u64, String)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if our_preview_running || device_path.is_empty() || !stream_owned_elsewhere(device_path) {
        *last = None;
        return CameraUse::default();
    }
    let rdev = fs::metadata(device_path).map(|m| m.rdev()).unwrap_or(0);
    let app_name = match last.as_ref() {
        Some((cached, name)) if *cached == rdev => name.clone(),
        _ => {
            let name = pick_name(&processes_using(rdev)).unwrap_or_else(|| "another app".to_string());
            *last = Some((rdev, name.clone()));
            name
        }
    };
    CameraUse { in_use: true, app_name }
}

/// True when another file handle owns the capture stream. Any other failure
/// (camera unplugged, no permission) counts as not in use: we can't tell.
fn stream_owned_elsewhere(device_path: &str) -> bool {
    let Ok(fd) = v4l2::open_node(device_path) else {
        return false;
    };
    let mut req: v4l2::RequestBuffers = v4l2::zeroed();
    req.count = 1;
    req.kind = v4l2::BUF_TYPE_VIDEO_CAPTURE;
    req.memory = v4l2::MEMORY_MMAP;
    match v4l2::ioctl(&fd, v4l2::VIDIOC_REQBUFS, &mut req) {
        Err(libc::EBUSY) => true,
        Ok(()) => {
            // We now own the stream: give it straight back so others can start.
            req.count = 0;
            let _ = v4l2::ioctl(&fd, v4l2::VIDIOC_REQBUFS, &mut req);
            false
        }
        Err(_) => false,
    }
}

/// The short names (/proc/<pid>/comm) of other processes holding the device
/// open. Processes can exit while we look, so every failure just skips one.
fn processes_using(rdev: u64) -> Vec<String> {
    let own_pid = std::process::id();
    let Ok(procs) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in procs.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        // Other users' processes can't be read; that's fine.
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        let holds_device = fds.flatten().any(|fd| {
            // Cheap filter first: only links into /dev can be the camera.
            let is_dev = fs::read_link(fd.path()).is_ok_and(|target| target.starts_with("/dev/"));
            is_dev && fs::metadata(fd.path()).is_ok_and(|m| m.file_type().is_char_device() && m.rdev() == rdev)
        });
        if holds_device {
            if let Ok(comm) = fs::read_to_string(entry.path().join("comm")) {
                let comm = comm.trim();
                if !comm.is_empty() && !names.iter().any(|n| n == comm) {
                    names.push(comm.to_string());
                }
            }
        }
    }
    names
}

fn is_pipewire(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("pipewire") || lower == "wireplumber"
}

/// The most useful name to show. PipeWire and WirePlumber keep cameras open for
/// the apps they serve, so a real app's name wins.
fn pick_name(names: &[String]) -> Option<String> {
    if let Some(app) = names.iter().find(|n| !is_pipewire(n)) {
        return Some(app.clone());
    }
    (!names.is_empty()).then(|| "PipeWire".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prefers_a_real_app() {
        assert_eq!(pick_name(&names(&["pipewire", "zoom"])), Some("zoom".into()));
        assert_eq!(pick_name(&names(&["wireplumber", "pipewire", "obs"])), Some("obs".into()));
        assert_eq!(pick_name(&names(&["pipewire-media-", "wireplumber"])), Some("PipeWire".into()));
        assert_eq!(pick_name(&[]), None);
    }

    #[test]
    fn not_in_use_without_a_camera() {
        assert_eq!(query("", false), CameraUse::default());
        assert_eq!(query("/dev/video0", true), CameraUse::default());
        assert_eq!(query("/nonexistent/video0", false), CameraUse::default());
        // A file that isn't a video node: REQBUFS fails with ENOTTY, not EBUSY.
        assert_eq!(query("/dev/null", false), CameraUse::default());
    }

    #[test]
    fn finds_processes_by_device_number() {
        // A child with /dev/null as its input stands in for an app holding the camera.
        let Ok(mut child) = std::process::Command::new("sleep").arg("5").stdin(std::process::Stdio::null()).spawn()
        else {
            return; // No `sleep` here: nothing to test with.
        };
        let rdev = fs::metadata("/dev/null").unwrap().rdev();
        let found = processes_using(rdev);
        let _ = child.kill();
        let _ = child.wait();
        assert!(found.iter().any(|n| n == "sleep"), "{found:?}");
        // A device nobody has open.
        assert!(processes_using(u64::MAX).is_empty());
    }
}
