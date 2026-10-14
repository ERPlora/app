// Evita una segunda consola en Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    erplora_tauri_lib::run()
}
