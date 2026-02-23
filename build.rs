fn main() {
    // Link macOS frameworks required by mac-notification-sys
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }

    // Ensure Cargo re-runs the build when embedded asset files change.
    // rust-embed uses a proc macro to embed these at compile time, but Cargo
    // only tracks .rs file changes by default. Without these directives,
    // modifying a dockerfile or template won't trigger a rebuild.
    println!("cargo:rerun-if-changed=dockerfiles");
    println!("cargo:rerun-if-changed=templates");
}
