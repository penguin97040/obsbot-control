//! Small operating-system services the rest of the app needs.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(windows)]
pub use windows::*;

/// Held for the life of the app so a second copy can tell this one is running.
pub struct InstanceGuard {
    #[allow(dead_code)]
    inner: Box<dyn std::any::Any>,
}

impl InstanceGuard {
    pub fn new(inner: impl std::any::Any) -> Self {
        InstanceGuard { inner: Box::new(inner) }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod other {
    use super::InstanceGuard;
    pub fn single_instance() -> Option<InstanceGuard> {
        Some(InstanceGuard::new(()))
    }
    pub fn os_version() -> String {
        std::env::consts::OS.to_string()
    }
}
#[cfg(not(any(target_os = "linux", windows)))]
pub use other::*;
