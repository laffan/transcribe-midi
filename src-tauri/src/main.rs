// Suppress the console window on Windows release builds. Windows is out of scope, but
// this is the standard Tauri entry point and costs nothing to keep correct.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    unplugged_lib::run()
}
