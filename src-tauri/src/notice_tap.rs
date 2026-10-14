//! A tap on a system notice, kept for the page until it claims it (hub#2360).
//!
//! hub#2305 made a tap open the screen the notice is about, through the notification plugin's
//! `actionPerformed`. Two places never got there:
//!
//! * **The computer.** The plugin's desktop `show()` hands the notice to `notify-rust` on a spawned
//!   task and drops the handle: no click ever comes back, so a click brings the app to the front
//!   wherever it was left. Here the shell shows the notice itself, on a thread of its own that waits
//!   for the click.
//! * **Android, cold.** A tap that has to START the app fires `actionPerformed` while the plugin is
//!   loading, before any page listens, and the plugin drops an event nobody is listening to. The
//!   Android plugin keeps that tap (`take_notice_tap`) and the page claims it once it is up. When the
//!   system had killed the process but the task is still in recents, the activity comes back with
//!   its old launcher intent and the tap arrives through `MainActivity.onNewIntent`, which feeds the
//!   same box. Android hands the intent a task was born from back at every return of the process,
//!   so the tap on it (id and content) is remembered in the app's preferences and counts once; a
//!   tap through `onNewIntent` is delivered once and is never lost to that older one.
//!
//! In both, the tap lands in [`KeptNoticeTap`] with the screen the notice was sent with, and the page
//! claims it through `erplora_take_notice_tap` — at boot, and every time [`NOTICE_TAPPED_EVENT`]
//! says one is waiting. The page may be a new one whose memory of ids is empty, which is why the tap
//! carries its screen and not only its id.

use std::sync::Mutex;

#[cfg(desktop)]
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The event that tells the page a tap is waiting to be claimed. It carries nothing: the tap itself
/// is handed over once, by `erplora_take_notice_tap`, so two listeners never open a screen twice.
#[cfg(desktop)]
pub const NOTICE_TAPPED_EVENT: &str = "erplora://notice-tapped";

/// A tap on a notice: its id and the screen it was sent with, when it names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeTap {
    pub id: i32,
    pub path: Option<String>,
}

impl NoticeTap {
    /// The tap in the shape of the plugin's `actionPerformed`, so the page reads both the same way.
    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({ "notification": { "id": self.id, "extra": { "path": self.path } } })
    }

    /// The tap that started the app on Android: the notice's id and the JSON the plugin stored the
    /// notice as, whose `extra.path` is the screen. A JSON that does not read still opens the app —
    /// only without a screen of its own.
    pub fn from_launch(id: i32, notification: Option<&str>) -> Self {
        let path = notification
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|notice| notice["extra"]["path"].as_str().map(str::to_owned));
        Self { id, path }
    }
}

/// The last tap nobody has claimed yet. The last one wins: it is the notice the person just chose.
#[derive(Debug, Default)]
pub struct KeptNoticeTap(Mutex<Option<NoticeTap>>);

impl KeptNoticeTap {
    #[cfg_attr(mobile, allow(dead_code))] // on the phone Kotlin keeps the tap
    pub fn keep(&self, tap: NoticeTap) {
        // A poisoned lock only means another thread panicked holding a plain value: still usable.
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tap);
    }

    /// Hands the tap over, once.
    pub fn take(&self) -> Option<NoticeTap> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
    }
}

/// A click on a notice on the computer: keep the tap, bring the window to the front and tell the
/// page a tap is waiting. A notice without an id still brings the window up — it just leads nowhere.
#[cfg(desktop)]
pub fn on_click<R: Runtime>(app: &AppHandle<R>, tap: Option<NoticeTap>) {
    if let Some(tap) = tap {
        app.state::<KeptNoticeTap>().keep(tap);
    }
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("notice: a notice was clicked but there is no window to bring up");
        return;
    };
    // Each step on its own: a window the OS will not raise must still be told a tap is waiting.
    if let Err(e) = window.unminimize() {
        log::warn!("notice: the window could not be restored ({e})");
    }
    if let Err(e) = window.show() {
        log::warn!("notice: the window could not be shown ({e})");
    }
    if let Err(e) = window.set_focus() {
        log::warn!("notice: the window could not take the focus ({e})");
    }
    if let Err(e) = app.emit_to("main", NOTICE_TAPPED_EVENT, ()) {
        log::warn!("notice: the page could not be told a tap is waiting ({e})");
    }
}

/// The host of a notice's own link, `erplora://notice?id=<id>&path=<screen>` (hub#2409).
const NOTICE_LINK_HOST: &str = "notice";

/// The link a notice opens when it is clicked, wherever it is clicked from. Windows does not bring a
/// click on a notice in the Action Center back to the thread that showed it, so the notice carries
/// its tap in a link of the app's own scheme and a click launches the app with it (hub#2409). A
/// notice without an id still links to the app: the click brings the window up.
#[cfg_attr(not(windows), allow(dead_code))] // only Windows writes it into the notice
pub fn notice_link(tap: Option<&NoticeTap>) -> String {
    let mut link = tauri::Url::parse(&format!("{}://{NOTICE_LINK_HOST}", crate::DEEP_LINK_SCHEME))
        .expect("the notice link is a constant URL");
    if let Some(tap) = tap {
        let mut query = link.query_pairs_mut();
        query.append_pair("id", &tap.id.to_string());
        if let Some(path) = &tap.path {
            query.append_pair("path", path);
        }
    }
    link.to_string()
}

/// The notice click among the arguments the app was launched with: `Some(tap)` for a notice link —
/// whose tap is `None` when the link names no id that reads — and `None` for anything else.
///
/// Anyone can hand the app an `erplora://` link, so what it names is only kept when it is a screen
/// of the app (a path of its own, never another address); the page checks it again before it goes.
pub fn click_from_args<I: IntoIterator<Item = String>>(args: I) -> Option<Option<NoticeTap>> {
    args.into_iter().find_map(|arg| click_from_link(&arg))
}

fn click_from_link(raw: &str) -> Option<Option<NoticeTap>> {
    let link = tauri::Url::parse(raw.trim()).ok()?;
    // The host of a non-special scheme is opaque: the parser does not lowercase it.
    if link.scheme() != crate::DEEP_LINK_SCHEME || !link.host_str()?.eq_ignore_ascii_case(NOTICE_LINK_HOST) {
        return None;
    }
    let value = |name: &str| link.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    let Some(id) = value("id").and_then(|id| id.parse::<i32>().ok()) else {
        return Some(None);
    };
    let path = value("path").filter(|path| path.starts_with('/') && !path.starts_with("//"));
    Some(Some(NoticeTap { id, path }))
}

/// The Windows notice: its words, and the link a click opens — on screen or later in the Action
/// Center, with the app open or closed — as a protocol activation.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn toast_xml(title: &str, body: &str, link: &str) -> String {
    format!(
        r#"<toast launch="{}" activationType="protocol"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        xml_escape(link),
        xml_escape(title),
        xml_escape(body),
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// A launch of the app by a notice link is a click on that notice: it is answered like any other
/// click, and `true` says so. Anything else is left to the caller.
#[cfg(desktop)]
pub fn answer_link<R: Runtime, I: IntoIterator<Item = String>>(app: &AppHandle<R>, args: I) -> bool {
    let Some(tap) = click_from_args(args) else { return false };
    on_click(app, tap);
    true
}

/// The computer's notification centre: shows a notice and waits for its click.
#[cfg(desktop)]
pub use desktop::deliver;

/// Where a notice is delivered: `Ok(true)` when the person clicked it, `Ok(false)` when it went by.
#[cfg(desktop)]
pub type Deliver<R> = fn(&AppHandle<R>, &str, &str, Option<&NoticeTap>) -> Result<bool, String>;

/// Shows a notice on the computer and waits for its click on a thread of its own — the wait lasts
/// as long as the notice is on screen. The thread is handed back so a test can wait for the answer.
/// Never fails upwards: a notice that does not go out is logged and the order behind it carries on.
#[cfg(desktop)]
pub fn show<R: Runtime>(
    app: AppHandle<R>,
    title: String,
    body: String,
    tap: Option<NoticeTap>,
    deliver: Deliver<R>,
) -> Option<std::thread::JoinHandle<()>> {
    // One parked thread per notice still on screen or in the notification centre: a small stack
    // keeps a busy kitchen's afternoon of notices cheap.
    let spawned = std::thread::Builder::new().name("notice".into()).stack_size(256 * 1024).spawn(move || {
        match deliver(&app, &title, &body, tap.as_ref()) {
            Ok(true) => on_click(&app, tap),
            Ok(false) => {}
            Err(e) => log::warn!("notify: the platform could not show «{title}» ({e}) — carrying on"),
        }
    });
    if let Err(e) = &spawned {
        log::warn!("notify: no thread to show the notice on ({e}) — carrying on");
    }
    spawned.ok()
}

#[cfg(target_os = "macos")]
mod desktop {
    use mac_notification_sys::{Notification, NotificationResponse};
    use tauri::{AppHandle, Runtime};

    /// `notify-rust` cannot be used here: on macOS it only waits for an answer when the notice has
    /// buttons, so a plain notice comes back at once with no click. `wait_for_click` is the call that
    /// waits, and it lives one crate down.
    pub fn deliver<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str, _tap: Option<&super::NoticeTap>) -> Result<bool, String> {
        // Whose notice it is. A development run is not a bundle, so it borrows the Terminal's, as the
        // notification plugin does. It can be set once per process; the error of a second call only
        // says it already was.
        let bundle = if tauri::is_dev() { "com.apple.Terminal".to_owned() } else { app.config().identifier.clone() };
        let _ = mac_notification_sys::set_application(&bundle);
        Notification::new()
            .title(title)
            .message(body)
            .wait_for_click(true)
            .send()
            .map(|response| opens(&response))
            .map_err(|e| e.to_string())
    }

    /// A click on the notice itself opens it; letting it go by does not.
    pub fn opens(response: &NotificationResponse) -> bool {
        matches!(response, NotificationResponse::Click)
    }
}

/// Windows: the notice carries its own link (hub#2409). A click on the notice while it is on screen
/// could be heard by the thread that showed it, but once it moves to the Action Center (about seven
/// seconds later) that thread has had its answer — «expired» — and a later click there has nobody to
/// go to, even less with the app closed. So the notice is a protocol activation of
/// [`notice_link`]: every click launches the app with the link, single-instance hands it to the app
/// that is already open (`on_second_launch`) and a closed app reads it at boot. Nothing waits here.
#[cfg(windows)]
mod desktop {
    use tauri::{AppHandle, Runtime};
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    /// Windows' own id to show a notice under when the app is not installed: a build run from
    /// `target/` has none of its own (the notification plugin's own rule, via `notify-rust`).
    const POWERSHELL_APP_ID: &str = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    /// `Ok(false)` always: the click does not come back here but through the notice's link.
    pub fn deliver<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str, tap: Option<&super::NoticeTap>) -> Result<bool, String> {
        let xml = XmlDocument::new().map_err(|e| e.to_string())?;
        xml.LoadXml(&HSTRING::from(super::toast_xml(title, body, &super::notice_link(tap))))
            .map_err(|e| e.to_string())?;
        let notice = ToastNotification::CreateToastNotification(&xml).map_err(|e| e.to_string())?;
        // The Store copy is a package and shows notices under its own identity; one installed by the
        // `.exe`/`.msi` does it under the identifier its Start-menu entry carries.
        let notifier = if crate::distribution_channel() == "msstore" {
            ToastNotificationManager::CreateToastNotifier()
        } else if installed() {
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app.config().identifier.as_str()))
        } else {
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(POWERSHELL_APP_ID))
        }
        .map_err(|e| e.to_string())?;
        notifier.Show(&notice).map_err(|e| e.to_string())?;
        Ok(false)
    }

    /// The Start-menu identity only exists for the installed app; a build run from `target/` has
    /// none and the toast would not show under it (the notification plugin's own rule).
    fn installed() -> bool {
        use std::path::MAIN_SEPARATOR as SEP;
        let Ok(exe) = tauri::utils::platform::current_exe() else { return false };
        let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
        !(dir.ends_with(&format!("{SEP}target{SEP}debug")) || dir.ends_with(&format!("{SEP}target{SEP}release")))
    }
}

#[cfg(all(desktop, not(target_os = "macos"), not(windows)))]
mod desktop {
    use notify_rust::{Notification, NotificationResponse};
    use tauri::{AppHandle, Runtime};

    pub fn deliver<R: Runtime>(_app: &AppHandle<R>, title: &str, body: &str, _tap: Option<&super::NoticeTap>) -> Result<bool, String> {
        let mut notice = Notification::new();
        // Linux reports a click on the body only when the notice offers the `default` action.
        notice.summary(title).body(body).auto_icon().action("default", "");
        let handle = notice.show().map_err(|e| e.to_string())?;
        let mut opened = false;
        handle
            .wait_for_response(|response: &NotificationResponse| opened = opens(response))
            .map_err(|e| e.to_string())?;
        Ok(opened)
    }

    /// A click on the body opens the notice: the `default` action.
    pub fn opens(response: &NotificationResponse) -> bool {
        match response {
            NotificationResponse::Default => true,
            NotificationResponse::Action(key) => key == "default",
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tauri::Listener;

    #[test]
    fn a_kept_tap_reads_like_the_plugins_own() {
        let tap = NoticeTap { id: 1_727_000_042, path: Some("/m/whatsapp_inbox/42".into()) };
        assert_eq!(
            tap.payload(),
            serde_json::json!({ "notification": { "id": 1_727_000_042, "extra": { "path": "/m/whatsapp_inbox/42" } } })
        );
    }

    #[test]
    fn a_tap_without_a_screen_still_carries_its_id() {
        let tap = NoticeTap { id: 7, path: None };
        assert_eq!(tap.payload()["notification"]["id"], 7);
        assert!(tap.payload()["notification"]["extra"]["path"].is_null());
    }

    #[test]
    fn the_tap_that_started_the_app_brings_its_screen_back() {
        let json = r#"{"id":9,"title":"New booking","extra":{"path":"/m/appointments"}}"#;
        assert_eq!(
            NoticeTap::from_launch(9, Some(json)),
            NoticeTap { id: 9, path: Some("/m/appointments".into()) }
        );
    }

    #[test]
    fn a_launch_whose_notice_does_not_read_still_opens_the_app() {
        for broken in [None, Some("not json"), Some(r#"{"extra":{"path":42}}"#), Some("{}")] {
            assert_eq!(NoticeTap::from_launch(9, broken), NoticeTap { id: 9, path: None }, "{broken:?}");
        }
    }

    #[test]
    fn a_kept_tap_is_handed_over_once() {
        let kept = KeptNoticeTap::default();
        kept.keep(NoticeTap { id: 1, path: None });
        assert_eq!(kept.take(), Some(NoticeTap { id: 1, path: None }));
        assert_eq!(kept.take(), None);
    }

    #[test]
    fn the_last_tap_wins() {
        let kept = KeptNoticeTap::default();
        kept.keep(NoticeTap { id: 1, path: Some("/m/kds".into()) });
        kept.keep(NoticeTap { id: 2, path: Some("/m/appointments".into()) });
        assert_eq!(kept.take(), Some(NoticeTap { id: 2, path: Some("/m/appointments".into()) }));
    }

    #[cfg(desktop)]
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        // `mock_context(noop_assets())` and NOT `generate_context!()` (see `connectivity.rs`).
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        app.manage(KeptNoticeTap::default());
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
            .build()
            .expect("mock window");
        app
    }

    #[cfg(desktop)]
    #[test]
    fn a_click_keeps_the_tap_and_tells_the_page() {
        let app = mock_app();
        let heard = Arc::new(Mutex::new(0));
        let counter = heard.clone();
        app.listen_any(NOTICE_TAPPED_EVENT, move |_| *counter.lock().unwrap() += 1);

        on_click(app.handle(), Some(NoticeTap { id: 3, path: Some("/m/kds".into()) }));

        assert_eq!(*heard.lock().unwrap(), 1, "the page was not told a tap is waiting");
        assert_eq!(
            app.state::<KeptNoticeTap>().take(),
            Some(NoticeTap { id: 3, path: Some("/m/kds".into()) })
        );
    }

    #[cfg(desktop)]
    #[test]
    fn a_click_on_a_notice_without_an_id_keeps_nothing() {
        let app = mock_app();
        app.state::<KeptNoticeTap>().keep(NoticeTap { id: 1, path: Some("/m/kds".into()) });
        on_click(app.handle(), None);
        // The earlier tap is still the one waiting: an id-less notice does not erase it.
        assert_eq!(app.state::<KeptNoticeTap>().take(), Some(NoticeTap { id: 1, path: Some("/m/kds".into()) }));
    }

    #[cfg(desktop)]
    fn show_and_wait(delivered: Deliver<tauri::test::MockRuntime>) -> (Option<NoticeTap>, usize) {
        let app = mock_app();
        let heard = Arc::new(Mutex::new(0));
        let counter = heard.clone();
        app.listen_any(NOTICE_TAPPED_EVENT, move |_| *counter.lock().unwrap() += 1);
        let tap = Some(NoticeTap { id: 6, path: Some("/m/appointments".into()) });
        if let Some(thread) = show(app.handle().clone(), "New booking".into(), "Ana · 10:00".into(), tap, delivered) {
            thread.join().expect("the notice thread panicked");
        }
        let kept = app.state::<KeptNoticeTap>().take();
        let times = *heard.lock().unwrap();
        (kept, times)
    }

    #[cfg(desktop)]
    #[test]
    fn a_click_on_the_shown_notice_keeps_its_tap_and_tells_the_page() {
        let (kept, heard) = show_and_wait(|_, _, _, _| Ok(true));
        assert_eq!(kept, Some(NoticeTap { id: 6, path: Some("/m/appointments".into()) }));
        assert_eq!(heard, 1, "the page was not told a tap is waiting");
    }

    #[cfg(desktop)]
    #[test]
    fn the_platform_is_handed_the_tap_its_notice_links_to() {
        // A platform that gets the click back through the notice's link (Windows, hub#2409) needs
        // the tap to write that link: here it answers «clicked» only when it was given the right one.
        let (kept, _) = show_and_wait(|_, _, _, tap| Ok(tap == Some(&NoticeTap { id: 6, path: Some("/m/appointments".into()) })));
        assert_eq!(kept, Some(NoticeTap { id: 6, path: Some("/m/appointments".into()) }));
    }

    #[cfg(desktop)]
    #[test]
    fn a_notice_that_goes_by_unclicked_opens_nothing() {
        assert_eq!(show_and_wait(|_, _, _, _| Ok(false)), (None, 0));
    }

    #[cfg(desktop)]
    #[test]
    fn a_notice_the_platform_cannot_show_opens_nothing() {
        assert_eq!(show_and_wait(|_, _, _, _| Err("no notification centre".into())), (None, 0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_a_mac_only_a_click_on_the_notice_opens_it() {
        use mac_notification_sys::NotificationResponse as R;
        assert!(desktop::opens(&R::Click));
        for other in [R::None, R::CloseButton("Close".into()), R::ActionButton("x".into())] {
            assert!(!desktop::opens(&other), "{other:?}");
        }
    }

    #[cfg(all(desktop, not(target_os = "macos"), not(windows)))]
    #[test]
    fn on_linux_a_click_on_the_body_opens_the_notice() {
        use notify_rust::{CloseReason, NotificationResponse as R};
        assert!(desktop::opens(&R::Default));
        assert!(desktop::opens(&R::Action("default".into())));
        for other in [R::Action("other".into()), R::Closed(CloseReason::Dismissed), R::Closed(CloseReason::Expired)] {
            assert!(!desktop::opens(&other), "{other:?}");
        }
    }

    // ── hub#2409: the click that comes back through the notice's own link ────────────────────────

    fn launched_with(link: &str) -> Vec<String> {
        vec![r"C:\Program Files\ERPlora\ERPlora.exe".into(), link.into()]
    }

    #[test]
    fn a_notice_link_brings_its_tap_back() {
        for path in ["/m/appointments", "/m/whatsapp_inbox/42?q=ña b&x=1#top", "/"] {
            let tap = NoticeTap { id: 1_727_000_042, path: Some(path.into()) };
            assert_eq!(click_from_args(launched_with(&notice_link(Some(&tap)))), Some(Some(tap)), "{path}");
        }
        let tap = NoticeTap { id: 7, path: None };
        assert_eq!(click_from_args(launched_with(&notice_link(Some(&tap)))), Some(Some(tap)));
    }

    #[test]
    fn a_notice_without_an_id_still_links_to_the_app() {
        assert_eq!(click_from_args(launched_with(&notice_link(None))), Some(None));
    }

    #[test]
    fn only_a_notice_link_is_a_notice_click() {
        for arg in [
            "erplora://hub/demo.a.erplora.com",
            "https://notice?id=1&path=/m/kds",
            "erplora://notices?id=1",
            "erplora://notice.evil.com?id=1",
            "--autostart",
            "",
        ] {
            assert_eq!(click_from_args(launched_with(arg)), None, "{arg}");
        }
        assert_eq!(click_from_args(vec![r"C:\ERPlora.exe".to_owned()]), None);
    }

    #[test]
    fn a_notice_link_in_capitals_is_still_a_notice_link() {
        assert_eq!(
            click_from_args(launched_with("ERPLORA://NOTICE?id=3&path=%2Fm%2Fkds")),
            Some(Some(NoticeTap { id: 3, path: Some("/m/kds".into()) }))
        );
    }

    #[test]
    fn a_link_whose_id_does_not_read_still_brings_the_app_up() {
        for link in [
            "erplora://notice?id=abc&path=%2Fm%2Fkds",
            "erplora://notice?id=2147483648&path=%2Fm%2Fkds",
            "erplora://notice?path=%2Fm%2Fkds",
        ] {
            assert_eq!(click_from_args(launched_with(link)), Some(None), "{link}");
        }
    }

    #[test]
    fn a_link_leads_only_to_a_screen_of_the_app() {
        // Anyone can hand the app an `erplora://` link: what it names is never a way out of the app.
        for path in ["https://evil.example/m/kds", "//evil.example/m/kds", "m/kds", ""] {
            let link = format!("erplora://notice?id=3&path={}", path.replace('/', "%2F").replace(':', "%3A"));
            assert_eq!(click_from_args(launched_with(&link)), Some(Some(NoticeTap { id: 3, path: None })), "{path}");
        }
    }

    #[test]
    fn the_notice_opens_its_link_when_clicked_anywhere() {
        let tap = NoticeTap { id: 6, path: Some("/m/appointments".into()) };
        let xml = toast_xml("New booking", "Ana · 10:00", &notice_link(Some(&tap)));
        assert!(xml.starts_with(r#"<toast launch="erplora://notice?id=6&amp;path=%2Fm%2Fappointments" activationType="protocol">"#), "{xml}");
        assert!(xml.contains("<text>New booking</text><text>Ana · 10:00</text>"), "{xml}");
    }

    #[test]
    fn the_notice_s_words_cannot_break_its_markup() {
        let xml = toast_xml(r#"<b>Tom & "Jerry's"</b>"#, "a < b > c", "erplora://notice");
        assert!(xml.contains("<text>&lt;b&gt;Tom &amp; &quot;Jerry&apos;s&quot;&lt;/b&gt;</text>"), "{xml}");
        assert!(xml.contains("<text>a &lt; b &gt; c</text>"), "{xml}");
    }

    #[cfg(desktop)]
    #[test]
    fn a_notice_link_keeps_its_tap_and_tells_the_page() {
        let app = mock_app();
        let heard = Arc::new(Mutex::new(0));
        let counter = heard.clone();
        app.listen_any(NOTICE_TAPPED_EVENT, move |_| *counter.lock().unwrap() += 1);
        let tap = NoticeTap { id: 8, path: Some("/m/kds".into()) };

        assert!(answer_link(app.handle(), launched_with(&notice_link(Some(&tap)))));

        assert_eq!(*heard.lock().unwrap(), 1, "the page was not told a tap is waiting");
        assert_eq!(app.state::<KeptNoticeTap>().take(), Some(tap));
    }

    #[cfg(desktop)]
    #[test]
    fn a_launch_that_is_not_a_notice_link_is_left_alone() {
        let app = mock_app();
        let heard = Arc::new(Mutex::new(0));
        let counter = heard.clone();
        app.listen_any(NOTICE_TAPPED_EVENT, move |_| *counter.lock().unwrap() += 1);

        assert!(!answer_link(app.handle(), launched_with("erplora://hub/demo.a.erplora.com")));

        assert_eq!(*heard.lock().unwrap(), 0);
        assert_eq!(app.state::<KeptNoticeTap>().take(), None);
    }
}
