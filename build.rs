//! Embed Windows application icon (Explorer / taskbar for the .exe).

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // Path relative to this package (workspace root for fancontrol-rs binary).
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            // Don't hard-fail non-Windows cross builds or missing tools mid-CI.
            println!("cargo:warning=winresource icon embed failed: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/logo.svg");
}
