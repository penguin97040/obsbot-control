// Embeds the Windows icon, manifest and version information. Nothing to do elsewhere.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/icon.ico");
    #[cfg(windows)]
    windows_resources();
    // Windows resolves the exe's own DLL imports (d3d11, dwmapi, mfplat...) from
    // System32 only, so a DLL planted beside the exe (in Downloads, say) is never
    // loaded. Needs Windows 10 1607 or later; older Windows ignores the flag.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}

/// Fills the version into res/app.rc and compiles it. The version string goes in as
/// text rather than a /D macro, because quotes don't survive the resource compiler's
/// command line.
#[cfg(windows)]
fn windows_resources() {
    use std::path::Path;

    let version = env!("CARGO_PKG_VERSION");
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("res");
    // The generated file lives elsewhere, so it names the icon and manifest by full path.
    let quoted = |name: &str| format!("\"{}\"", res.join(name).display().to_string().replace('\\', "\\\\"));
    let template = std::fs::read_to_string(res.join("app.rc")).expect("could not read res/app.rc");
    let rc = template
        .replace("\"icon.ico\"", &quoted("icon.ico"))
        .replace("\"app.manifest\"", &quoted("app.manifest"))
        .replace("APP_VERSION_MAJOR", &major.to_string())
        .replace("APP_VERSION_MINOR", &minor.to_string())
        .replace("APP_VERSION_PATCH", &patch.to_string())
        .replace("APP_VERSION_STRING", &format!("\"{version}\""));
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo")).join("app.rc");
    std::fs::write(&out, rc).expect("could not write the resource file");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("could not embed the Windows resources");
}
