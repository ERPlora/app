//! «Start on login».
//!
//! The print queue lives in the hub and this device drains it: if nobody opened the
//! app, the tickets wait. On a dedicated desktop till, starting with the session guarantees there
//! is always a print host. Opt-in, OFF by default — and the OS owns the state (LaunchAgent /
//! registry / autostart dir), we keep NO persistence of our own. Desktop only: the plugin does not
//! support Android/iOS, and a tablet has its own idea of "login".

use crate::ShellError;

/// The OS-level switch behind the setting, as a seam: the real one writes a LaunchAgent — a unit
/// test that flipped it would leave the developer's machine starting a till at login.
trait AutostartSwitch {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

/// Applies the desired state and answers with what the OS says NOW — read back, never assumed. A
/// toggle that showed ON while the OS said OFF would be a till that never starts, discovered the
/// morning the first unprinted ticket is.
fn apply_autostart(switch: &dyn AutostartSwitch, enabled: bool) -> Result<bool, String> {
    switch.set_enabled(enabled)?;
    switch.is_enabled()
}

/// The real switch: `tauri-plugin-autostart`, which talks to the LaunchAgent (macOS), the Run
/// registry key (Windows) or the autostart dir (Linux).
#[cfg(desktop)]
struct OsAutostart<'a>(&'a tauri::AppHandle);

#[cfg(desktop)]
impl AutostartSwitch for OsAutostart<'_> {
    fn is_enabled(&self) -> Result<bool, String> {
        use tauri_plugin_autostart::ManagerExt;
        self.0.autolaunch().is_enabled().map_err(|e| e.to_string())
    }
    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        let launcher = self.0.autolaunch();
        if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
    }
}

/// `autostart_is_enabled` — the CURRENT state, straight from the OS. On mobile the command
/// answers an error on purpose: the setting must not render there at all, and a silent `false`
/// would let it.
#[tauri::command]
pub(crate) fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return OsAutostart(&app).is_enabled().map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

/// `autostart_enable` — turns «Start on login» on and answers with the state read back.
#[tauri::command]
pub(crate) fn autostart_enable(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return apply_autostart(&OsAutostart(&app), true).map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

/// `autostart_disable` — turns it off; answers with the state read back.
#[tauri::command]
pub(crate) fn autostart_disable(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return apply_autostart(&OsAutostart(&app), false).map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── «Start on login» — OFF by default, the OS owns the state ─────────────────────────────
    //
    // Behind a seam because the real switch writes a LaunchAgent / registry key / autostart
    // file: a unit test that ENABLED it would leave the developer's machine starting a till at
    // login. The fake proves the command logic; the real impl is one delegation to the plugin.

    struct FakeAutostart {
        enabled: std::cell::Cell<bool>,
        /// A switch whose `enable()` succeeds but changes nothing — how a sandboxed macOS build
        /// or a broken LaunchAgent dir actually behaves.
        stuck: bool,
    }

    impl FakeAutostart {
        fn off() -> Self {
            Self { enabled: std::cell::Cell::new(false), stuck: false }
        }
    }

    impl AutostartSwitch for FakeAutostart {
        fn is_enabled(&self) -> Result<bool, String> {
            Ok(self.enabled.get())
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), String> {
            if !self.stuck {
                self.enabled.set(enabled);
            }
            Ok(())
        }
    }

    #[test]
    fn autostart_is_off_until_somebody_turns_it_on() {
        // The whole sequence: default disabled → enable → is_enabled → disable.
        let switch = FakeAutostart::off();
        assert_eq!(switch.is_enabled(), Ok(false), "opt-in means the default is OFF");
        assert_eq!(apply_autostart(&switch, true), Ok(true));
        assert_eq!(switch.is_enabled(), Ok(true));
        assert_eq!(apply_autostart(&switch, false), Ok(false));
        assert_eq!(switch.is_enabled(), Ok(false));
    }

    #[test]
    fn the_reported_state_is_what_the_os_says_not_what_was_asked() {
        // No persistence of our own: the OS owns the state, so the answer must be
        // read BACK after the change. A switch that quietly failed to change must be reported as
        // it is — a toggle that shows ON while the OS says OFF is a till that never starts.
        let stuck = FakeAutostart { enabled: std::cell::Cell::new(false), stuck: true };
        assert_eq!(apply_autostart(&stuck, true), Ok(false));
    }
}
