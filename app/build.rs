//! Embeds the OZBEAT icon into the Windows executable (Explorer, taskbar pin).

fn main() {
    println!("cargo:rerun-if-changed=assets/ozbeat.ico");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/ozbeat.ico");
        resource.set("ProductName", "OZBEAT");
        resource.set("FileDescription", "OZBEAT music visualizer");
        if let Err(e) = resource.compile() {
            // A missing resource compiler only costs the file icon, not the build.
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
