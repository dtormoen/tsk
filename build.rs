fn main() {
    // Link macOS frameworks required by mac-notification-sys
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
