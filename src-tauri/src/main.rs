// Keeps the Windows console hidden on release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    linkamp_lib::run()
}
