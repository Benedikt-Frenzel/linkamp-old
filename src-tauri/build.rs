fn main() {
    // Debian's runtime image has the versioned ALSA library used by CPAL but
    // not the unversioned development symlink.
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,-l:libasound.so.2");
    tauri_build::build()
}
