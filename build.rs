// Embeds the Windows icon, manifest and version information. Nothing to do elsewhere.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/icon.ico");
    #[cfg(windows)]
    {
        let version = env!("CARGO_PKG_VERSION");
        let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
        let (major, minor, patch) = (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        );
        let macros = [
            format!("APP_VERSION_MAJOR={major}"),
            format!("APP_VERSION_MINOR={minor}"),
            format!("APP_VERSION_PATCH={patch}"),
            format!("APP_VERSION_STRING=\\\"{version}\\\""),
        ];
        embed_resource::compile("res/app.rc", &macros)
            .manifest_required()
            .expect("could not embed the Windows resources");
    }
}
