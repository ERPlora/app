// Declara los comandos del plugin para que `tauri-plugin` autogenere sus permisos ACL
// (`allow-check-permissions`, `allow-request-permissions`) y compile el módulo Android.
// `leave_app` (hub#1906): the way out when the shell holds the Back button and nothing is left.
// `open_app_settings` (hub#1886): the way back once Android stops showing a permission dialog.
// `keep_listening` (hub#2307): keeps the app running with the screen off so the notices still arrive.
const COMMANDS: &[&str] = &[
    "check_permissions",
    "request_permissions",
    "leave_app",
    "open_app_settings",
    "keep_listening",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
