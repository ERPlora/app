//! Device identity (X-Device-Id).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ShellError;

/// Stable per-installation device id, sent as `X-Device-Id` at login (one session per device).
const DEVICE_ID_FILE: &str = "device.id";

/// Device identity the frontend sends at login (one session per device), as `X-Client-Type` +
/// `X-Device-Id` + `X-Device-Platform`.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceContext {
    pub id: String,
    pub client_type: String,
    pub platform: String,
    /// Where THIS binary came from: `play`, `msstore` or `direct`.
    ///
    /// Only the shell can tell. The web is served by the hub, so a flag of its own would be the
    /// same for every installation and could not tell a Play copy from one installed by hand.
    /// Here it is baked in at compile time: the store jobs of the release workflow export
    /// `ERPLORA_DISTRIBUTION`, and everything else (Windows installer, `.deb`, AppImage) stays
    /// `direct`, which is exactly the copy that must keep announcing new versions.
    pub distribution: String,
}

/// The channel this binary came through. Each platform resolves it differently **because CI
/// builds them differently**:
///
/// - **Android**: the AAB is its own build and only goes to Play, so the mark is baked in at
///   compile time (`ERPLORA_DISTRIBUTION=play` in that job). `option_env!` rather than `env!`:
///   without the variable the build does not fail, it declares `direct`.
/// - **Windows**: the MSIX is packaged **from the same `.exe`** as the regular installer (the
///   workflow does that on purpose so the app is built only once), so a compile-time flag would
///   be the same for both channels. It has to be checked at run time: a packaged app runs from
///   `WindowsApps`, one installed through the `.exe`/`.msi` does not.
///
/// The bias of the errors is deliberate. Erring towards `direct` leaves a store copy with one
/// notice too many (ugly, and review catches it); erring towards `msstore` would leave a counter
/// till never learning that there is a new version, which is the problem this exists to avoid.
/// When in doubt, `direct`.
pub(crate) fn distribution_channel() -> &'static str {
    if let Some(forced) = option_env!("ERPLORA_DISTRIBUTION") {
        return match forced {
            "play" => "play",
            "msstore" => "msstore",
            _ => "direct",
        };
    }
    #[cfg(target_os = "windows")]
    {
        // `GetCurrentPackageFullName` would be the canonical check, but it needs the `windows`
        // crate for a single call. The path is the same observable fact: the MSIX runtime mounts
        // the app under `%ProgramFiles%\WindowsApps\<identity>`, where no installer ever lands.
        if std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_lowercase().contains("windowsapps"))
            .unwrap_or(false)
        {
            return "msstore";
        }
    }
    "direct"
}

/// Reads (or creates and persists) a stable per-installation device id in `app_data_dir`.
/// It survives webview cache wipes: it identifies **this** installation of the shell.
fn ensure_device_id(cache_dir: &Path) -> Result<String, ShellError> {
    let path = cache_dir.join(DEVICE_ID_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(cache_dir).map_err(|e| ShellError::Io(e.to_string()))?;
    std::fs::write(&path, &id).map_err(|e| ShellError::Io(e.to_string()))?;
    Ok(id)
}

/// Returns the device identity the page sends at login.
#[tauri::command]
pub(crate) fn device_context(app: tauri::AppHandle) -> Result<DeviceContext, ShellError> {
    use tauri::Manager;
    let cache_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| ShellError::Io(e.to_string()))?;
    let id = ensure_device_id(&cache_dir)?;
    // Without the `android` branch the shell announced itself as `hub-desktop`/`desktop` FROM A
    // PHONE (seen on the emulator, API 37), so the server could not tell a tablet from a counter
    // till, neither in the session list nor in the one-session-per-device rule.
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "desktop"
    };
    let client_type = if cfg!(any(target_os = "android", target_os = "ios")) {
        "hub-mobile"
    } else {
        "hub-desktop"
    };
    Ok(DeviceContext {
        id,
        // The server's client taxonomy; the concrete platform travels in X-Device-Platform.
        client_type: client_type.to_string(),
        platform: platform.to_string(),
        distribution: distribution_channel().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tempdir;

    // ── Device identity ──────────────────────────────────────────────────────────────────────

    #[test]
    fn ensure_device_id_is_stable_across_calls() {
        let dir = tempdir();
        let a = ensure_device_id(&dir).expect("first");
        let b = ensure_device_id(&dir).expect("second");
        assert_eq!(a, b);
        assert!(!a.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
