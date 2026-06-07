//! Build script: embeds the application icon into `terminal-manager.exe` on
//! Windows so the window, taskbar, and Explorer show the brand icon.
//!
//! This is best-effort. If the icon file is missing or the platform resource
//! compiler is unavailable, the build still succeeds — the executable simply
//! falls back to the default icon.

fn main() {
    #[cfg(windows)]
    {
        let icon = "packaging/app.ico";
        println!("cargo:rerun-if-changed={icon}");
        if std::path::Path::new(icon).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon);
            if let Err(e) = res.compile() {
                println!("cargo:warning=skipped embedding app icon: {e}");
            }
        }
    }
}
