//! System notices: showing one, and handing the page the tap it was not there to hear.

use crate::notice_tap;

/// `erplora_notify` — a SYSTEM notice (the OS one, not a toast inside the app).
///
/// That is what it is for: warning when **nobody is looking at the screen**. The case behind it is
/// the kitchen order — an order comes in and the kitchen has to find out even with the tablet on
/// another view or locked. A toast inside the app does not reach anyone there.
///
/// The SHELL exposes it because in Tauri the shell **is** the hardware bridge.
///
/// `id` is the handle of the tap: the notification plugin reports a tap with the id of
/// the notice on Android and iOS alike, and the page remembers which screen that id leads to. A
/// page older than this shell sends none and the notice goes out under the plugin's own id.
///
/// `path` is the screen itself, for the taps that do not reach the page that sent the notice: on
/// the computer the shell shows the notice itself and keeps the click. On Android it travels in
/// `extra`, but the plugin never stores the notice on the intent of its tap, which comes back with
/// the id only: Kotlin writes the screen down under the id before the notice
/// goes out, and hands it back with the tap.
///
/// **Never fails upwards.** If the user denied the permission or the platform cannot show it, it is
/// logged and life goes on: a notice that does not go out cannot bring down the order behind it.
#[tauri::command]
pub(crate) fn erplora_notify(app: tauri::AppHandle, title: String, body: String, id: Option<i64>, path: Option<String>) {
    // The notification plugin shows a desktop notice and drops its handle: no click would come back.
    #[cfg(desktop)]
    // The thread is left to run on its own: it lasts as long as the notice does.
    let _ = notify_on_desktop(app, title, body, id, path, notice_tap::deliver);
    #[cfg(mobile)]
    {
        use tauri_plugin_erplora_android::ErploraAndroidExt as _;
        // Before the notice is out: a tap right after it must find its screen.
        remember_notice_target(id, path.as_deref(), |id, path| app.erplora_android().remember_notice(id, path));
        if let Err(e) = notice_builder(&app, &title, &body, id, path.as_deref()).show() {
            eprintln!("notify: the platform could not show «{title}» ({e}) — carrying on");
        }
    }
}

/// The notice `erplora_notify` shows, under the page's id when the plugin can hold it and with the
/// screen it leads to when it names one.
#[cfg_attr(desktop, allow(dead_code))]
fn notice_builder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    title: &str,
    body: &str,
    id: Option<i64>,
    path: Option<&str>,
) -> tauri_plugin_notification::NotificationBuilder<R> {
    use tauri_plugin_notification::NotificationExt;

    let mut builder = app.notification().builder().title(title).body(body);
    if let Some(id) = notice_id(id) {
        builder = builder.id(id);
    }
    if let Some(path) = path {
        builder = builder.extra("path", path);
    }
    builder
}

/// `erplora_take_notice_tap` — the tap the page was not there to hear, handed over once:
/// a click on the computer, or the tap that started the app on Android. `null` when there is none.
#[tauri::command]
pub(crate) fn erplora_take_notice_tap(app: tauri::AppHandle) -> Option<serde_json::Value> {
    take_notice_tap(&app)
}

/// `erplora_notify` on the computer, with the platform's delivery handed in: the notice carries
/// back its id, when it has one the platform can hold, and its screen.
#[cfg(desktop)]
fn notify_on_desktop<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    title: String,
    body: String,
    id: Option<i64>,
    path: Option<String>,
    deliver: notice_tap::Deliver<R>,
) -> Option<std::thread::JoinHandle<()>> {
    notice_tap::show(app, title, body, notice_id(id).map(|id| notice_tap::NoticeTap { id, path }), deliver)
}

/// Kotlin's answer to «which tap started the app», in the shape the page reads every tap in.
fn launch_tap_payload<E: std::fmt::Display>(
    answer: Result<Option<tauri_plugin_erplora_android::LaunchNoticeTap>, E>,
) -> Option<serde_json::Value> {
    match answer {
        Ok(tap) => tap.map(|tap| notice_tap::NoticeTap::from_launch(tap.id, tap.notification.as_deref(), tap.path.as_deref()).payload()),
        Err(e) => {
            log::warn!("notice: the tap that started the app could not be read ({e})");
            None
        }
    }
}

fn take_notice_tap<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<serde_json::Value> {
    use tauri::Manager;
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;

    if let Some(tap) = app.state::<notice_tap::KeptNoticeTap>().take() {
        return Some(tap.payload());
    }
    launch_tap_payload(app.erplora_android().take_notice_tap())
}

/// Writes down the screen a notice leads to under the number it goes out with, through
/// `remember`. A notice under no number the plugin can hold goes out under the plugin's own, which
/// nothing was written under. Never fails upwards: a screen that is not written down only means
/// that the tap opens the app without it.
#[cfg_attr(desktop, allow(dead_code))]
fn remember_notice_target<E: std::fmt::Display>(
    id: Option<i64>,
    path: Option<&str>,
    remember: impl FnOnce(i32, Option<&str>) -> Result<(), E>,
) {
    let Some(id) = notice_id(id) else { return };
    if let Err(e) = remember(id, path) {
        log::warn!("notice: the screen of notice {id} could not be written down ({e})");
    }
}

/// The id the plugin can hold (`i32`), or none: an id out of range must not stop the notice, only
/// lose its tap's destination.
fn notice_id(id: Option<i64>) -> Option<i32> {
    id.and_then(|id| i32::try_from(id).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::app_with_kept_tap;

    // ── The id a tap on a notice comes back with ─────────────────────────────────────────────────
    //
    // The plugin takes an `i32`. An id the page sends out of that range must not stop the notice:
    // it goes out under the plugin's own id, and only the tap loses its destination.

    #[test]
    fn a_notice_keeps_the_id_the_page_gave_it() {
        assert_eq!(notice_id(Some(1_727_000_000)), Some(1_727_000_000));
        assert_eq!(notice_id(Some(i64::from(i32::MIN))), Some(i32::MIN));
        assert_eq!(notice_id(None), None);
    }

    #[test]
    fn an_id_the_plugin_cannot_hold_still_lets_the_notice_out() {
        assert_eq!(notice_id(Some(i64::from(i32::MAX) + 1)), None);
        assert_eq!(notice_id(Some(i64::MIN)), None);
    }

    /// The builder only shows its fields through `Debug`; that is what the tap is matched on.
    fn built_notice(id: Option<i64>) -> String {
        built_notice_to(id, None)
    }

    fn built_notice_to(id: Option<i64>, path: Option<&str>) -> String {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_notification::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        format!("{:?}", notice_builder(app.handle(), "New booking", "Ana · 10:00", id, path))
    }

    // ── The screen travels with the notice ───────────────────────────────────────────────────────
    //
    // A tap that STARTS the app on Android reaches a page that never saw the notice go out, so its
    // memory of ids is empty. The notice carries its screen in `extra`, which Android hands back
    // with the tap.

    #[test]
    fn the_notice_carries_the_screen_its_tap_opens() {
        let built = built_notice_to(Some(7), Some("/m/appointments"));
        // On `extra` itself: the mock app's Debug also lists Tauri's own `path` plugin.
        assert!(built.contains(r#"extra: {"path": String("/m/appointments")}"#), "{built}");
    }

    #[test]
    fn a_notice_that_leads_nowhere_carries_no_screen() {
        let built = built_notice_to(Some(7), None);
        assert!(built.contains("extra: {}"), "{built}");
    }


    #[test]
    fn the_page_claims_the_kept_tap_once() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        app.state::<notice_tap::KeptNoticeTap>()
            .keep(notice_tap::NoticeTap { id: 5, path: Some("/m/kds".into()) });
        assert_eq!(
            take_notice_tap(app.handle()),
            Some(serde_json::json!({ "notification": { "id": 5, "extra": { "path": "/m/kds" } } }))
        );
        assert_eq!(take_notice_tap(app.handle()), None);
    }

    /// What the page keeps after `erplora_notify` shows a notice on the computer and the person
    /// clicks it: the notice goes through the same path as the command, with a clicking platform.
    #[cfg(desktop)]
    fn clicked_desktop_notice(id: Option<i64>, path: Option<&str>) -> Option<notice_tap::NoticeTap> {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let shown = notify_on_desktop(app.handle().clone(), "New booking".into(), "Ana".into(), id, path.map(Into::into), |_, _, _, _| Ok(true));
        shown.expect("no thread for the notice").join().expect("the notice thread panicked");
        app.state::<notice_tap::KeptNoticeTap>().take()
    }

    #[cfg(desktop)]
    #[test]
    fn a_desktop_notice_carries_its_id_and_screen_back() {
        assert_eq!(
            clicked_desktop_notice(Some(7), Some("/m/kds")),
            Some(notice_tap::NoticeTap { id: 7, path: Some("/m/kds".into()) })
        );
        assert_eq!(clicked_desktop_notice(Some(7), None), Some(notice_tap::NoticeTap { id: 7, path: None }));
        // No id the platform can hold, nothing to open: the click only brings the window up.
        assert_eq!(clicked_desktop_notice(None, Some("/m/kds")), None);
        assert_eq!(clicked_desktop_notice(Some(i64::from(i32::MAX) + 1), Some("/m/kds")), None);
    }

    #[test]
    fn the_tap_that_started_the_app_reads_like_any_other() {
        let tap = tauri_plugin_erplora_android::LaunchNoticeTap {
            id: 4,
            notification: Some(r#"{"id":4,"extra":{"path":"/m/appointments"}}"#.into()),
            path: None,
        };
        assert_eq!(
            launch_tap_payload::<String>(Ok(Some(tap))),
            Some(serde_json::json!({ "notification": { "id": 4, "extra": { "path": "/m/appointments" } } }))
        );
        assert_eq!(launch_tap_payload::<String>(Ok(None)), None);
        assert_eq!(launch_tap_payload(Err("plugin gone")), None);
    }

    // ── On Android a tap comes back with the notice's number only ────────────────────────────────

    #[test]
    fn a_kept_tap_without_its_notice_opens_the_screen_the_app_wrote_down() {
        // The notification plugin never stores the notice on the tap's intent: the kitchen's notice
        // comes back as its number, and the screen is the one Kotlin wrote down as it went out.
        let tap = tauri_plugin_erplora_android::LaunchNoticeTap {
            id: 8,
            notification: None,
            path: Some("/kitchen/display".into()),
        };
        assert_eq!(
            launch_tap_payload::<String>(Ok(Some(tap))),
            Some(serde_json::json!({ "notification": { "id": 8, "extra": { "path": "/kitchen/display" } } }))
        );
    }

    #[test]
    fn a_notice_writes_down_its_screen_under_the_number_it_goes_out_with() {
        let mut written = Vec::new();
        remember_notice_target(Some(1_727_000_008), Some("/kitchen/display"), |id, path| {
            written.push((id, path.map(str::to_owned)));
            Ok::<(), String>(())
        });
        assert_eq!(written, vec![(1_727_000_008, Some("/kitchen/display".to_owned()))]);
    }

    #[test]
    fn a_notice_without_a_number_the_plugin_holds_writes_nothing_down() {
        // Its tap comes back under the plugin's own number, which no screen was written under.
        for id in [None, Some(i64::from(i32::MAX) + 1)] {
            remember_notice_target(id, Some("/kitchen/display"), |_, _| -> Result<(), String> {
                panic!("{id:?} was written down")
            });
        }
    }

    #[test]
    fn a_screen_that_cannot_be_written_down_does_not_stop_the_notice() {
        // Never fails upwards, like the notice itself: the order behind it goes on.
        remember_notice_target(Some(7), Some("/kitchen/display"), |_, _| Err("plugin gone"));
    }

    #[test]
    fn nothing_kept_is_nothing_to_claim() {
        let app = app_with_kept_tap();
        assert_eq!(take_notice_tap(app.handle()), None);
    }

    #[test]
    fn the_notice_goes_out_under_the_id_its_tap_comes_back_with() {
        let built = built_notice(Some(1_727_000_042));
        assert!(built.contains("id: 1727000042"), "{built}");
        assert!(built.contains("New booking") && built.contains("Ana · 10:00"), "{built}");
    }

    #[test]
    fn a_notice_without_an_id_still_goes_out_with_its_words() {
        let built = built_notice(None);
        assert!(!built.contains("id: 1727000042"), "{built}");
        assert!(built.contains("New booking") && built.contains("Ana · 10:00"), "{built}");
    }
}
