// Declares the plugin's commands so `tauri-plugin` generates their ACL permissions
// (`allow-check-permissions`, `allow-request-permissions`, …) and builds the Android module.
// `leave_app` (hub#1906): the way out when the shell holds the Back button and nothing is left.
// `open_app_settings` (hub#1886): the way back once Android stops showing a permission dialog.
// `tts_available`, `tts_speak`, `tts_stop` (round 2026-11-04a): the system voice that reads the
// assistant's answers aloud, since the Android WebView has no `speechSynthesis`.
const COMMANDS: &[&str] = &[
    "check_permissions",
    "request_permissions",
    "leave_app",
    "open_app_settings",
    "tts_available",
    "tts_speak",
    "tts_stop",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
