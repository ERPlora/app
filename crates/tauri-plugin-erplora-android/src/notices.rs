//! The taps on the app's notices: the one that started the app and the ones that wait for the page.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

/// The tap on a notice that STARTED the app: the notice's id, the JSON the notification plugin
/// stored it as, whose `extra.path` is the screen it leads to, and the screen the app wrote down
/// when it showed it — the plugin never stores that JSON, so `path` is what brings the screen back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LaunchNoticeTap {
    pub id: i32,
    #[serde(default)]
    pub notification: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// The Kotlin command that writes down the screen a notice leads to, as it goes out.
/// Only Rust calls it: it is in neither `build.rs` nor the default permissions.
pub const REMEMBER_NOTICE_COMMAND: &str = "rememberNotice";

/// The Kotlin command Rust hands the channel through which Kotlin says a tap waits for
/// the page. Only Rust calls it: it is in neither `build.rs` nor the default permissions.
pub const WATCH_NOTICE_TAPS_COMMAND: &str = "watchNoticeTaps";

/// The notice's number and the screen it leads to, under the keys Kotlin reads.
#[derive(Debug, Serialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct RememberNoticeArgs<'a> {
    id: i32,
    path: Option<&'a str>,
}

/// `rememberNotice` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type RememberNoticeAnswer = serde::de::IgnoredAny;

/// `watchNoticeTaps` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type WatchNoticeTapsAnswer = serde::de::IgnoredAny;

/// The channel Kotlin calls back on when a tap waits for the page, under the key Kotlin reads.
#[cfg(target_os = "android")]
#[derive(Serialize)]
struct WatchNoticeTapsArgs {
    handler: tauri::ipc::Channel,
}

/// The wire shape of `takeNoticeTap`: `tap` absent or `null` when the app was not started by one.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct NoticeTapAnswer {
    #[serde(default)]
    tap: Option<LaunchNoticeTap>,
}

impl<R: Runtime> ErploraAndroid<R> {
    /// The tap on a notice that started the app, handed over once.
    ///
    /// The notification plugin reports that tap while it is still loading, before the page has a
    /// listener, and the event is lost; Kotlin keeps it instead. Off Android there is no such launch.
    pub fn take_notice_tap(&self) -> Result<Option<LaunchNoticeTap>, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<NoticeTapAnswer>("takeNoticeTap", Empty {})
                .map(|answer| answer.tap)
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(None)
    }

    /// Writes down the screen notice `id` leads to, as it goes out. The notification
    /// plugin brings a tap back with the notice's number only (it never stores the notice on the
    /// tap's intent), so this is what lets the tap open its screen, also when it starts the app.
    /// Off Android a tap carries its screen itself: there is nothing to write down.
    pub fn remember_notice(&self, id: i32, path: Option<&str>) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<RememberNoticeAnswer>(REMEMBER_NOTICE_COMMAND, RememberNoticeArgs { id, path })
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (id, path);
            Ok(())
        }
    }

    /// `on_tap` runs every time a tap on a notice waits for a page that already
    /// listens: the notification plugin's own report of it says nothing the page can open. The
    /// shell tells the page, which claims the tap with `take_notice_tap`. Off Android there is no
    /// such tap to watch.
    pub fn watch_notice_taps<F: Fn() + Send + Sync + 'static>(&self, on_tap: F) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            use tauri::ipc::Channel;
            return self
                .0
                .run_mobile_plugin::<WatchNoticeTapsAnswer>(
                    WATCH_NOTICE_TAPS_COMMAND,
                    WatchNoticeTapsArgs {
                        handler: Channel::new(move |_| {
                            on_tap();
                            Ok(())
                        }),
                    },
                )
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = on_tap;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── The tap that started the app ─────────────────────────────────────────────────────────────
    //
    // The notification plugin reports it while it is still loading, before the page listens, and the
    // event is lost. Kotlin keeps it and the shell asks for it once — three places that have to agree
    // on one command name and one answer shape, none of which the compiler sees.

    #[test]
    fn the_kept_launch_tap_reads_with_and_without_a_tap() {
        let kept: NoticeTapAnswer =
            serde_json::from_str(r#"{"tap":{"id":7,"notification":"{\"extra\":{\"path\":\"/m/kds\"}}"}}"#)
                .expect("a kept tap reads");
        assert_eq!(
            kept.tap,
            Some(LaunchNoticeTap { id: 7, notification: Some(r#"{"extra":{"path":"/m/kds"}}"#.into()), path: None })
        );
        let none: NoticeTapAnswer = serde_json::from_str(r#"{"tap":null}"#).expect("no tap reads");
        assert_eq!(none.tap, None);
        let bare: NoticeTapAnswer = serde_json::from_str("{}").expect("an empty answer reads");
        assert_eq!(bare.tap, None);
    }

    #[test]
    fn kotlin_keeps_the_launch_tap_and_hands_it_over_once() {
        const NOTICE_TAPS_KT: &str = include_str!("../android/src/main/java/com/erplora/android/NoticeTaps.kt");
        let production = production_rust();
        assert!(
            production.contains("run_mobile_plugin::<NoticeTapAnswer>(\"takeNoticeTap\", Empty {})"),
            "Rust no longer asks Kotlin for the kept tap by the name Kotlin answers to"
        );
        let before = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin takeNoticeTap is not a @Command");
        let command = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").nth(1).expect("no Kotlin takeNoticeTap");
        let body = command.split("\n    }").next().unwrap_or_default();
        assert!(body.contains("NoticeTaps.take()"), "takeNoticeTap does not hand over the tap the box kept");
        let load = PLUGIN_KT.split("override fun load(webView: WebView)").nth(1).expect("Kotlin does not look at the launch");
        let load = load.split("\n    }").next().unwrap_or_default();
        assert!(
            load.contains("NoticeTaps.pageLoading(activity, activity.intent)"),
            "load does not hand the box the intent that started the app"
        );
        // The memory has to outlive the process: back through the icon after the system killed it,
        // the task's intent is the old tap again.
        assert!(
            NOTICE_TAPS_KT.contains("NoticeTapBox(PreferencesMemory(context.applicationContext))"),
            "the box no longer remembers the kept tap outside the process"
        );
        assert!(
            NOTICE_TAPS_KT.contains("prefs.edit().putString(LAST_KEPT, key).apply()"),
            "the kept tap is not written to the app's preferences"
        );
    }

    // ── A tap comes back without its notice ─────────────────────────────────────────────────────
    //
    // The notification plugin never stores the notice on the intent of its tap (`sourceJson` is
    // never set, tauri-apps/plugins-workspace#3550): its `actionPerformed` says neither the number
    // nor the screen, and the tap Kotlin kept had no screen either — the page stayed where it was.
    // The app writes each notice's screen down as it goes out, hands it back with the tap, and tells
    // the page a tap waits while it listens: names and keys the compiler does not see.

    #[test]
    fn the_screen_of_a_notice_crosses_to_kotlin_under_the_keys_kotlin_reads() {
        assert_eq!(
            serde_json::to_value(RememberNoticeArgs { id: 8, path: Some("/kitchen/display") }).expect("serializable"),
            serde_json::json!({ "id": 8, "path": "/kitchen/display" })
        );
        assert_eq!(
            serde_json::to_value(RememberNoticeArgs { id: 9, path: None }).expect("serializable"),
            serde_json::json!({ "id": 9, "path": null })
        );
        let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
        let command = format!("fun {REMEMBER_NOTICE_COMMAND}(invoke: Invoke)");
        let before = code.split(&command).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin {REMEMBER_NOTICE_COMMAND} is not a @Command");
        let body = code.split(&command).nth(1).expect("no Kotlin rememberNotice");
        let body = body.split("\n    }").next().unwrap_or_default();
        for literal in ["optInt(\"id\"", "getString(\"path\"", "NoticeTaps.remember(activity, id, path)"] {
            assert!(
                body.contains(literal),
                "{literal} is not in Kotlin rememberNotice — a tap on a notice would come back without its screen"
            );
        }
        assert!(
            production_rust().contains("run_mobile_plugin::<RememberNoticeAnswer>(REMEMBER_NOTICE_COMMAND,"),
            "Rust no longer hands Kotlin the screen of a notice by the name Kotlin answers to"
        );
    }

    #[test]
    fn the_kept_tap_brings_back_the_screen_the_app_wrote_down() {
        let kept: NoticeTapAnswer =
            serde_json::from_str(r#"{"tap":{"id":8,"notification":null,"path":"/kitchen/display"}}"#)
                .expect("a kept tap reads");
        assert_eq!(
            kept.tap,
            Some(LaunchNoticeTap { id: 8, notification: None, path: Some("/kitchen/display".into()) })
        );
        let command = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").nth(1).expect("no Kotlin takeNoticeTap");
        let body = command.split("\n    }").next().unwrap_or_default();
        assert!(
            body.contains("kept.put(\"path\", tap.path)"),
            "takeNoticeTap does not hand back the screen the app wrote down"
        );
    }

    #[test]
    fn kotlin_tells_rust_when_a_tap_waits_for_the_page() {
        const NOTICE_TAPS_KT: &str = include_str!("../android/src/main/java/com/erplora/android/NoticeTaps.kt");
        let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
        let command = format!("fun {WATCH_NOTICE_TAPS_COMMAND}(invoke: Invoke)");
        let before = code.split(&command).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin {WATCH_NOTICE_TAPS_COMMAND} is not a @Command");
        let body = code.split(&command).nth(1).expect("no Kotlin watchNoticeTaps");
        let body = body.split("\n    }").next().unwrap_or_default();
        for literal in ["parseArgs(WatchNoticeTapsArgs::class.java)", "NoticeTaps.onTapWaiting(activity)", "handler.send(JSObject())"] {
            assert!(body.contains(literal), "{literal} is not in Kotlin watchNoticeTaps — the page would never hear a tap with the app open");
        }
        assert!(code.contains("lateinit var handler: Channel"), "the channel does not cross to Kotlin under `handler`");
        let taps = without_kotlin_comments(NOTICE_TAPS_KT);
        for literal in [
            "NoticeTargets(PreferencesTargets(context.applicationContext))",
            "{ targets(context).pathOf(it) }",
            "box(context).onTapWaiting(listener)",
            "targets(context).remember(id, path)",
        ] {
            assert!(taps.contains(literal), "{literal} is not in NoticeTaps.kt");
        }
        let production = production_rust();
        assert!(
            production.contains("run_mobile_plugin::<WatchNoticeTapsAnswer>(") && production.contains("WATCH_NOTICE_TAPS_COMMAND,"),
            "Rust no longer hands Kotlin the channel by the name Kotlin answers to"
        );
        assert!(production.contains("handler: Channel::new("), "Rust hands Kotlin no channel to call back on");
    }

    #[test]
    fn no_page_can_write_down_a_notice_nor_watch_the_taps() {
        // Only Rust calls them: a command in `build.rs` gets an `allow-*` permission a capability
        // could grant, and a page that wrote down screens could send a tap anywhere.
        let build = include_str!("../build.rs");
        let permissions = include_str!("../permissions/default.toml");
        for name in [
            "remember_notice",
            "remember-notice",
            REMEMBER_NOTICE_COMMAND,
            "watch_notice_taps",
            "watch-notice-taps",
            WATCH_NOTICE_TAPS_COMMAND,
        ] {
            assert!(!build.contains(name), "build.rs declares {name}");
            assert!(!permissions.contains(name), "the default permissions grant {name}");
        }
    }
}
