//! Which page may drive this device: the hub the app is LINKED to, and no other (hub#2504).
//!
//! Tauri's ACL decides by PATTERN (`capabilities/default.json` → `https://*.erplora.com/*`), and a
//! pattern cannot name the one business this till belongs to: the hub is chosen at run time, from
//! the SaaS, long after the binary was built. So the ACL lets every page under erplora.com through
//! — another business's hub, the public website, the test SaaS — and this module is the second
//! gate, in front of the app's own commands, of the Android plugin's (hub#2642) and of the
//! notification plugin's (hub#2658): the printer, the drawer, the card reader, the way out to the
//! browser, the Downloads folder, Android's permission dialogs, the listening service, the way out
//! of the app and the taps on the notices answer only the page whose ORIGIN is the linked hub.
//!
//! The link itself needs the same care, or the gate is one navigation away from moot: any page
//! the window shows could navigate to its own address with `?shell=1` and become "the hub". So a
//! `?shell=1` navigation links the app only when it LEAVES an entry page (the SaaS, which chooses
//! the hub among the person's own) or lands on the hub already linked. The app links on its own
//! authority too — the hub it remembered, a link the system handed it — through [`HubLink::link`].
//!
//! The gate reads the page the WINDOW shows (`webview.url()`): Tauri checks its own ACL against the
//! request's `Origin`, but does not hand that origin to a command handler. And the window's URL
//! flips the moment a navigation STARTS, while the page that started it keeps running until the new
//! one commits — so a page could send the window to the linked hub and ask for the drawer right
//! after. Hence [`HubLink::landed`]: after a navigation to another origin, nothing drives the device
//! until the new page has finished loading.

use std::sync::Mutex;

use tauri::ipc::Invoke;
use tauri::{Runtime, Url};

/// What a page the ACL lets through may always ask for, linked or not: the device identity and the
/// escape hatch the onboarding needs (`capabilities/onboarding.json`), and the retry of the bundled
/// offline page (`capabilities/degraded.json`). Everything else belongs to the linked hub — a
/// command added tomorrow is closed until someone opens it here on purpose.
const OPEN_COMMANDS: [&str; 3] = ["device_context", "forget_hub", "shell_retry"];

/// The error code a page gets when it asks for a command that belongs to the linked hub.
pub const NOT_THE_LINKED_HUB: &str = "not_the_linked_hub";

/// May any page the ACL lets through call this command?
pub fn is_open_command(command: &str) -> bool {
    OPEN_COMMANDS.contains(&command)
}

/// The ORIGIN of `url`, serialized the way `hub.url` stores it. Opaque origins (`tauri://`, the
/// bundled page) serialize as `null`, which no hub origin ever equals.
fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

#[derive(Debug, Default)]
struct LinkState {
    /// The hub this app is linked to — the origin of `hub.url` — if any.
    linked: Option<String>,
    /// The origin of the page the window last went to: the one a `?shell=1` navigation leaves.
    showing: Option<String>,
    /// The origin of the page that last FINISHED loading. The window's URL reads as the new page
    /// from the moment a navigation starts (WKWebView's `URL`, WebView2's `Source`), while the page
    /// that started it keeps running until the new one commits: between the two, nothing drives.
    landed: Option<String>,
}

/// The hub this installation is linked to, and the page the window is on.
#[derive(Debug)]
pub struct HubLink {
    /// The pages that choose a hub for the person: the SaaS (and, in development, the page the
    /// override boots at).
    entries: Vec<String>,
    state: Mutex<LinkState>,
}

impl HubLink {
    /// `entries` are the origins a `?shell=1` navigation may leave; `showing` is where the window
    /// starts — no page ran before it, so it counts as landed from the start.
    pub fn new(entries: Vec<String>, showing: Option<&Url>) -> Self {
        Self {
            entries,
            state: Mutex::new(LinkState {
                linked: None,
                showing: showing.map(origin_of),
                landed: showing.map(origin_of),
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LinkState> {
        // A poisoned lock only means another thread panicked holding it; the two `Option`s inside
        // are still whole, so the gate keeps answering instead of taking the till down with it.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The app links this hub on its own authority: the one it remembered, or the one a link the
    /// system handed it points at (HUB_APP-F02, F03).
    pub fn link(&self, origin: &str) {
        self.state().linked = Some(origin.to_string());
    }

    /// No hub any more: «Change business», or the hub is gone (HUB_APP-F04, F05).
    pub fn forget(&self) {
        self.state().linked = None;
    }

    /// The hub this app is linked to, if any.
    pub fn linked(&self) -> Option<String> {
        self.state().linked.clone()
    }

    /// The window follows a navigation to `url`. Returns the origin to remember as `hub.url` when
    /// this navigation links the app (`?shell=1` on a hub of ours, from an entry page or onto the
    /// hub already linked); `None` otherwise.
    pub fn follow(&self, url: &Url) -> Option<String> {
        let captured = crate::shell_capture_origin(url);
        let mut state = self.state();
        let arriving = origin_of(url);
        let left = state.showing.replace(arriving.clone());
        if left.as_deref() != Some(arriving.as_str()) {
            // Another origin: the page that leaves may still run while the window already reads as
            // the one arriving. Nothing drives the device until the new page has finished loading.
            state.landed = None;
        }
        let origin = captured?;
        let from_an_entry = left.as_ref().is_some_and(|left| self.entries.contains(left));
        let already_linked = state.linked.as_deref() == Some(origin.as_str());
        if !from_an_entry && !already_linked {
            log::warn!(
                "shell: {origin} asks to be the linked hub from {}; only the SaaS chooses it",
                left.as_deref().unwrap_or("nowhere")
            );
            return None;
        }
        state.linked = Some(origin.clone());
        Some(origin)
    }

    /// The page at `url` finished loading (`PageLoadEvent::Finished`, wired by `open_main_window`):
    /// whatever page started the navigation is gone, and this one may drive if it is the hub.
    pub fn landed(&self, url: &Url) {
        self.state().landed = Some(origin_of(url));
    }

    /// May the page at `page` call the commands that belong to the linked hub? Only when it is the
    /// linked hub AND it is the page that finished loading — not one the window is merely on its
    /// way to.
    pub fn drives(&self, page: &Url) -> bool {
        let origin = origin_of(page);
        let state = self.state();
        state.linked.as_deref() == Some(origin.as_str())
            && state.landed.as_deref() == Some(origin.as_str())
    }
}

/// Puts the gate in front of the app's commands: `run` hands Tauri `guard(app_commands())`.
pub fn guard<R, F>(commands: F) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static
where
    R: Runtime,
    F: Fn(Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke: Invoke<R>| {
        if !is_open_command(invoke.message.command()) {
            if let Err(code) = linked_hub_only(&invoke) {
                invoke.resolver.reject(code);
                return true;
            }
        }
        commands(invoke)
    }
}

/// The gate for a command that belongs to the linked hub: `Err(NOT_THE_LINKED_HUB)` unless the
/// invoke comes from it, loaded. Fails closed: no link state, or a page whose address cannot be
/// read, drives nothing. The Android plugin runs it before every one of its commands — none of
/// them is open (hub#2642).
pub fn linked_hub_only<R: Runtime>(invoke: &Invoke<R>) -> Result<(), &'static str> {
    use tauri::Manager;
    let webview = invoke.message.webview_ref();
    let page = webview.url().ok();
    let link = webview.try_state::<HubLink>();
    if drives_from(link.as_deref(), page.as_ref()) {
        return Ok(());
    }
    log::warn!(
        "ipc: {} refused to {}: not the linked hub",
        invoke.message.command(),
        page.map(|page| origin_of(&page)).unwrap_or_else(|| "an unreadable page".to_string())
    );
    Err(NOT_THE_LINKED_HUB)
}

/// The Android plugin (`plugin:erplora-android`: Android's permissions, the listening service, the
/// way out of the app) as `run` registers it — behind the same gate as the app's own commands.
pub fn android_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_erplora_android::init(linked_hub_only::<R>)
}

/// The notification plugin (`plugin:notification`: hearing a tap on a notice) as `run` registers
/// it — behind the same gate (hub#2658). The plugin is a third party's and its `init` takes no
/// gate, so [`Gated`] wraps it.
pub fn notification_plugin<R: Runtime>() -> Gated<R> {
    Gated(tauri_plugin_notification::init())
}

/// A third-party plugin with [`linked_hub_only`] in front of every one of its commands: everything
/// else — its setup, the script it puts in every page, its hooks — is the plugin's own.
pub struct Gated<R: Runtime>(tauri::plugin::TauriPlugin<R>);

impl<R: Runtime> tauri::plugin::Plugin<R> for Gated<R> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn initialize(
        &mut self,
        app: &tauri::AppHandle<R>,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }

    // The plugin's script runs in the main frame only, which is what the trait's default
    // `initialization_script_2` makes of this one.
    fn initialization_script(&self) -> Option<String> {
        self.0.initialization_script()
    }

    fn window_created(&mut self, window: tauri::Window<R>) {
        self.0.window_created(window)
    }

    fn webview_created(&mut self, webview: tauri::Webview<R>) {
        self.0.webview_created(webview)
    }

    fn on_navigation(&mut self, webview: &tauri::Webview<R>, url: &Url) -> bool {
        self.0.on_navigation(webview, url)
    }

    fn on_page_load(&mut self, webview: &tauri::Webview<R>, payload: &tauri::webview::PageLoadPayload<'_>) {
        self.0.on_page_load(webview, payload)
    }

    fn on_event(&mut self, app: &tauri::AppHandle<R>, event: &tauri::RunEvent) {
        self.0.on_event(app, event)
    }

    fn extend_api(&mut self, invoke: Invoke<R>) -> bool {
        if let Err(code) = linked_hub_only(&invoke) {
            invoke.resolver.reject(code);
            // Answered: on a phone an unanswered `plugin:*` command falls through to the plugin's
            // Kotlin/Swift half, past the gate.
            return true;
        }
        self.0.extend_api(invoke)
    }
}

/// The page drives the device only when both are known: the link state, and the page the window
/// shows. Missing either one, nothing does.
fn drives_from(link: Option<&HubLink>, page: Option<&Url>) -> bool {
    match (link, page) {
        (Some(link), Some(page)) => link.drives(page),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAAS: &str = "https://erplora.com";
    const HUB: &str = "https://panaderia.a.erplora.com";

    fn url(raw: &str) -> Url {
        raw.parse().expect("test URL")
    }

    /// A fresh install on the SaaS onboarding, as `open_main_window` builds it.
    fn on_the_onboarding() -> HubLink {
        HubLink::new(vec![SAAS.to_string()], Some(&url("https://erplora.com/shell/")))
    }

    /// The till as it is every day after the first link: the SaaS sent the window to the hub, the
    /// hub's page finished loading.
    fn linked_to_the_hub() -> HubLink {
        let link = on_the_onboarding();
        link.follow(&url("https://panaderia.a.erplora.com/?shell=1"));
        link.landed(&url(HUB));
        link
    }

    // ── The gate: only the linked hub drives the device ────────────────────────────────────────

    #[test]
    fn the_linked_hub_drives_the_device_on_any_of_its_pages() {
        let link = linked_to_the_hub();
        assert!(link.drives(&url("https://panaderia.a.erplora.com/")));
        assert!(link.drives(&url("https://panaderia.a.erplora.com/m/sales?x=1#/pos")));
    }

    #[test]
    fn another_page_under_erplora_com_does_not_drive_the_device() {
        let link = linked_to_the_hub();
        for page in [
            "https://otronegocio.a.erplora.com/", // another business
            "https://www.erplora.com/",           // the public website
            "https://pre.erplora.com/",           // the test SaaS
            "https://erplora.com/shell/",         // the SaaS apex
            "http://panaderia.a.erplora.com/",    // the same host, another scheme
            "https://panaderia.a.erplora.com:8443/", // the same host, another port
            "http://127.0.0.1:5173/",             // a page served from this device
            "tauri://localhost/index.html",       // the bundled page
        ] {
            assert!(!link.drives(&url(page)), "{page} drives a till linked to {HUB}");
        }
    }

    #[test]
    fn nothing_drives_the_device_before_a_hub_is_linked() {
        let link = on_the_onboarding();
        assert!(!link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn forgetting_the_hub_takes_the_device_away_from_it() {
        let link = linked_to_the_hub();
        link.forget();
        assert_eq!(link.linked(), None);
        assert!(!link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_page_whose_address_cannot_be_read_drives_nothing() {
        let link = linked_to_the_hub();
        assert!(!drives_from(Some(&link), None));
        assert!(!drives_from(None, Some(&url(HUB))));
        assert!(drives_from(Some(&link), Some(&url(HUB))));
    }

    // ── The window's URL flips BEFORE the page that started the navigation is gone ─────────────
    //
    // WKWebView's `URL` (and WebView2's `Source`) is the provisional URL from the moment a
    // navigation starts, and the page that started it keeps running until the new one commits. So
    // a page that sends the window to the linked hub and asks for the drawer right after would be
    // read as the hub. The device answers only once the hub's page has finished loading.

    #[test]
    fn a_page_that_sends_the_window_to_the_hub_cannot_drive_it_until_the_hub_has_loaded() {
        let link = linked_to_the_hub();
        link.follow(&url("https://www.erplora.com/"));
        assert_eq!(link.follow(&url("https://panaderia.a.erplora.com/")), None);
        assert!(
            !link.drives(&url("https://panaderia.a.erplora.com/")),
            "the website still runs while the window already reads as the hub"
        );
        link.landed(&url("https://panaderia.a.erplora.com/"));
        assert!(link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_navigation_inside_the_hub_keeps_the_hardware() {
        let link = linked_to_the_hub();
        assert_eq!(link.follow(&url("https://panaderia.a.erplora.com/login")), None);
        assert!(link.drives(&url("https://panaderia.a.erplora.com/login")));
    }

    #[test]
    fn the_page_the_window_starts_on_drives_as_soon_as_it_is_linked() {
        // A cold start on the remembered hub: no page ran before it, nothing to wait for.
        let link =
            HubLink::new(vec![SAAS.to_string()], Some(&url("https://panaderia.a.erplora.com/")));
        link.link(HUB);
        assert!(link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_load_that_finishes_elsewhere_does_not_open_the_hub() {
        let link = linked_to_the_hub();
        link.follow(&url("https://www.erplora.com/"));
        link.follow(&url("https://panaderia.a.erplora.com/"));
        // The website's own load reports finished after the window already left it.
        link.landed(&url("https://www.erplora.com/"));
        assert!(!link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn only_identity_escape_and_retry_are_open_to_any_page() {
        for open in ["device_context", "forget_hub", "shell_retry"] {
            assert!(is_open_command(open), "{open} must stay reachable from the onboarding");
        }
        for owned in [
            "erplora_print",
            "erplora_test_print",
            "erplora_open_drawer",
            "erplora_discover_printers",
            "erplora_add_network_printer",
            "erplora_nfc_read",
            "open_external_url",
            "save_download",
            "print_document",
            "erplora_notify",
            "erplora_bridge_status",
            "autostart_enable",
            "a_command_added_tomorrow",
        ] {
            assert!(!is_open_command(owned), "{owned} is open to any page under erplora.com");
        }
    }

    // ── The link: who may make a hub "the hub" ─────────────────────────────────────────────────

    #[test]
    fn the_saas_links_the_hub_it_sends_the_window_to() {
        let link = on_the_onboarding();
        // The chooser's /shell/open/<id>/ 302s into the hub with the marker.
        assert_eq!(link.follow(&url("https://erplora.com/shell/open/7/")), None);
        assert_eq!(
            link.follow(&url("https://panaderia.a.erplora.com/?shell=1")),
            Some(HUB.to_string())
        );
        assert_eq!(link.linked().as_deref(), Some(HUB));
        // …and drives the device once its page has loaded.
        link.landed(&url("https://panaderia.a.erplora.com/"));
        assert!(link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_page_under_erplora_com_cannot_link_itself() {
        let link = linked_to_the_hub();
        // The hub page links out to the website; the website marks itself.
        assert_eq!(link.follow(&url("https://www.erplora.com/precios")), None);
        assert_eq!(link.follow(&url("https://www.erplora.com/?shell=1")), None);
        assert_eq!(link.linked().as_deref(), Some(HUB), "the website replaced the linked hub");
        assert!(!link.drives(&url("https://www.erplora.com/")));
    }

    #[test]
    fn another_business_cannot_take_the_till_by_marking_its_own_address() {
        let link = linked_to_the_hub();
        assert_eq!(link.follow(&url("https://otronegocio.a.erplora.com/")), None);
        assert_eq!(link.follow(&url("https://otronegocio.a.erplora.com/?shell=1")), None);
        assert_eq!(link.linked().as_deref(), Some(HUB));
    }

    #[test]
    fn the_linked_hub_may_carry_the_marker_again() {
        let link = linked_to_the_hub();
        assert_eq!(link.follow(&url("https://panaderia.a.erplora.com/login")), None);
        assert_eq!(
            link.follow(&url("https://panaderia.a.erplora.com/?shell=1")),
            Some(HUB.to_string())
        );
    }

    #[test]
    fn change_business_goes_back_through_the_saas() {
        let link = linked_to_the_hub();
        link.forget();
        link.follow(&url("https://erplora.com/shell/?choose=1"));
        assert_eq!(
            link.follow(&url("https://otronegocio.a.erplora.com/?shell=1")),
            Some("https://otronegocio.a.erplora.com".to_string())
        );
        assert!(!link.drives(&url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_marker_on_an_address_that_is_not_ours_links_nothing_even_from_the_saas() {
        let link = on_the_onboarding();
        assert_eq!(link.follow(&url("https://evil.com/?shell=1")), None);
        assert_eq!(link.linked(), None);
    }

    // ── Through the IPC: what a page actually gets back ────────────────────────────────────────
    //
    // A real `on_message` round trip on the mock runtime, with the gate in front of a dispatcher
    // that answers "ran" to any command. The request travels as the bundled page (`bundled_page`): the
    // mock context carries no ACL manifest, so Tauri's own pattern check stays out of the way and
    // what is measured is the gate alone — which reads the page the WINDOW shows (the request's
    // origin, the one Tauri's ACL checks, is not handed to a command handler).

    use tauri::test::{MockRuntime, INVOKE_KEY};

    struct Till {
        _app: tauri::App<MockRuntime>,
        window: tauri::WebviewWindow<MockRuntime>,
    }

    fn till_showing(page: &str, link: Option<HubLink>) -> Till {
        use tauri::Manager;
        let app = tauri::test::mock_builder()
            .invoke_handler(guard(|invoke: Invoke<MockRuntime>| {
                invoke.resolver.resolve("ran");
                true
            }))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        if let Some(link) = link {
            app.manage(link);
        }
        let window =
            tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(page)))
                .build()
                .expect("main window");
        Till { _app: app, window }
    }

    fn ask(till: &Till, command: &str) -> Result<serde_json::Value, serde_json::Value> {
        tauri::test::get_ipc_response(
            &till.window,
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: bundled_page(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<serde_json::Value>().expect("json answer"))
    }

    /// The origin a request from the app's own bundled page carries: `tauri://localhost` on macOS
    /// and Linux, `http://tauri.localhost` on Windows and Android (hub#2705). A hard-coded
    /// `tauri://` reads as a REMOTE page on Windows, and Tauri's ACL refuses it before the gate
    /// under test is ever asked.
    fn bundled_page() -> Url {
        crate::connectivity::bundled_page_url().expect("the bundled page's URL")
    }

    fn refused() -> Result<serde_json::Value, serde_json::Value> {
        Err(serde_json::Value::from(NOT_THE_LINKED_HUB))
    }

    #[test]
    fn the_website_is_refused_the_drawer_of_a_till_linked_to_a_hub() {
        let till = till_showing("https://www.erplora.com/", Some(linked_to_the_hub()));
        for command in [
            "erplora_open_drawer",
            "erplora_print",
            "erplora_test_print",
            "erplora_nfc_read",
            "erplora_bridge_status",
        ] {
            assert_eq!(ask(&till, command), refused(), "{command} ran for the website");
        }
    }

    #[test]
    fn another_business_is_refused_the_printer() {
        let till = till_showing("https://otronegocio.a.erplora.com/m/sales", Some(linked_to_the_hub()));
        assert_eq!(ask(&till, "erplora_print"), refused());
        assert_eq!(ask(&till, "save_download"), refused());
        assert_eq!(ask(&till, "open_external_url"), refused());
    }

    #[test]
    fn the_linked_hub_prints_and_opens_the_drawer() {
        let till = till_showing("https://panaderia.a.erplora.com/m/sales", Some(linked_to_the_hub()));
        assert_eq!(ask(&till, "erplora_print"), Ok("ran".into()));
        assert_eq!(ask(&till, "erplora_open_drawer"), Ok("ran".into()));
    }

    #[test]
    fn the_onboarding_still_gets_the_device_identity_and_the_escape_hatch() {
        let till = till_showing("https://erplora.com/shell/", Some(on_the_onboarding()));
        assert_eq!(ask(&till, "device_context"), Ok("ran".into()));
        assert_eq!(ask(&till, "forget_hub"), Ok("ran".into()));
        assert_eq!(ask(&till, "erplora_print"), refused());
    }

    #[test]
    fn the_hub_is_refused_while_the_page_that_sent_the_window_there_may_still_run() {
        use tauri::Manager;
        let link = linked_to_the_hub();
        link.follow(&url("https://otronegocio.a.erplora.com/"));
        link.follow(&url("https://panaderia.a.erplora.com/"));
        let till = till_showing("https://panaderia.a.erplora.com/", Some(link));
        assert_eq!(ask(&till, "erplora_open_drawer"), refused(), "the other business still runs");
        till._app.state::<HubLink>().landed(&url("https://panaderia.a.erplora.com/"));
        assert_eq!(ask(&till, "erplora_open_drawer"), Ok("ran".into()));
    }

    #[test]
    fn without_a_link_state_the_device_stays_closed() {
        // Fail closed: if the state never got managed, nothing but the open commands answer.
        let till = till_showing("https://panaderia.a.erplora.com/", None);
        assert_eq!(ask(&till, "erplora_open_drawer"), refused());
        assert_eq!(ask(&till, "device_context"), Ok("ran".into()));
    }

    // ── The Android plugin, through its own route (hub#2642) ──────────────────────────────────
    //
    // `plugin:erplora-android|…` never reaches the app's invoke handler: Tauri hands it to the
    // plugin's. The real plugin, as `run` registers it, with every command granted to the page the
    // way `capabilities/default.json` grants `erplora-android:default` to every page under
    // erplora.com — so what is measured is the second gate alone. On a computer the plugin answers
    // without Android (an empty permission map, nothing to leave), which is what "it ran" reads as.

    const ANDROID_COMMANDS: [&str; 5] =
        ["check_permissions", "request_permissions", "keep_listening", "leave_app", "open_app_settings"];

    fn till_with_the_android_plugin(page: &str, link: Option<HubLink>) -> Till {
        use tauri::Manager;
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        for command in ANDROID_COMMANDS {
            context.runtime_authority_mut().__allow_command(
                format!("plugin:erplora-android|{command}"),
                tauri::utils::acl::ExecutionContext::Local,
            );
        }
        let app = tauri::test::mock_builder()
            .plugin(android_plugin())
            .build(context)
            .expect("mock app");
        if let Some(link) = link {
            app.manage(link);
        }
        let window =
            tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(page)))
                .build()
                .expect("main window");
        Till { _app: app, window }
    }

    /// What each Android command answers on a computer when it runs, with the arguments the shell
    /// sends (`keep_listening` needs its `on`).
    fn ask_android(till: &Till, command: &str) -> Result<serde_json::Value, serde_json::Value> {
        let body = match command {
            "keep_listening" => serde_json::json!({ "on": false }),
            _ => serde_json::json!({}),
        };
        tauri::test::get_ipc_response(
            &till.window,
            tauri::webview::InvokeRequest {
                cmd: format!("plugin:erplora-android|{command}"),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: bundled_page(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<serde_json::Value>().expect("json answer"))
    }

    fn ran(command: &str) -> Result<serde_json::Value, serde_json::Value> {
        match command {
            "check_permissions" | "request_permissions" => Ok(serde_json::json!({})),
            _ => Ok(serde_json::Value::Null),
        }
    }

    #[test]
    fn the_website_is_refused_the_android_permissions_of_a_till_linked_to_a_hub() {
        let till = till_with_the_android_plugin("https://www.erplora.com/", Some(linked_to_the_hub()));
        for command in ANDROID_COMMANDS {
            assert_eq!(ask_android(&till, command), refused(), "{command} ran for the website");
        }
    }

    #[test]
    fn another_business_cannot_keep_the_till_listening_nor_send_the_person_out() {
        let till = till_with_the_android_plugin(
            "https://otronegocio.a.erplora.com/m/sales",
            Some(linked_to_the_hub()),
        );
        for command in ANDROID_COMMANDS {
            assert_eq!(ask_android(&till, command), refused(), "{command} ran for another business");
        }
    }

    #[test]
    fn the_linked_hub_asks_android_for_its_permissions() {
        let till = till_with_the_android_plugin(
            "https://panaderia.a.erplora.com/m/sales",
            Some(linked_to_the_hub()),
        );
        for command in ANDROID_COMMANDS {
            assert_eq!(ask_android(&till, command), ran(command), "{command} refused to the linked hub");
        }
    }

    #[test]
    fn the_hub_is_refused_android_permissions_while_the_page_that_sent_the_window_there_may_still_run() {
        use tauri::Manager;
        let link = linked_to_the_hub();
        link.follow(&url("https://otronegocio.a.erplora.com/"));
        link.follow(&url("https://panaderia.a.erplora.com/"));
        let till = till_with_the_android_plugin("https://panaderia.a.erplora.com/", Some(link));
        assert_eq!(ask_android(&till, "request_permissions"), refused(), "the other business still runs");
        till._app.state::<HubLink>().landed(&url("https://panaderia.a.erplora.com/"));
        assert_eq!(ask_android(&till, "request_permissions"), ran("request_permissions"));
    }

    #[test]
    fn without_a_link_state_android_permissions_stay_closed() {
        let till = till_with_the_android_plugin("https://panaderia.a.erplora.com/", None);
        for command in ANDROID_COMMANDS {
            assert_eq!(ask_android(&till, command), refused(), "{command} ran with no link state");
        }
    }

    #[test]
    fn the_onboarding_gets_no_android_permission() {
        let till = till_with_the_android_plugin("https://erplora.com/shell/", Some(on_the_onboarding()));
        for command in ANDROID_COMMANDS {
            assert_eq!(ask_android(&till, command), refused(), "{command} ran for the onboarding");
        }
    }

    // ── The notification plugin, through its own route (hub#2658) ─────────────────────────────
    //
    // `capabilities/default.json` grants `notification:allow-register-listener` to every page under
    // erplora.com, so the page can hear a tap on a notice (HUB_APP-F25). The plugin is a third
    // party's and takes no gate of its own: `run` registers it wrapped. On a phone,
    // `register_listener` is answered by the Kotlin/Swift half once the plugin's Rust handler lets
    // it through; on a computer nothing answers it, which is what "it reached the plugin" reads
    // as here. `is_permission_granted` is answered in Rust on a computer: it proves the wrapped
    // plugin still set itself up.

    fn till_with_the_notification_plugin(page: &str, link: Option<HubLink>) -> Till {
        use tauri::Manager;
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        for command in ["register_listener", "is_permission_granted"] {
            context.runtime_authority_mut().__allow_command(
                format!("plugin:notification|{command}"),
                tauri::utils::acl::ExecutionContext::Local,
            );
        }
        let app = tauri::test::mock_builder()
            .plugin(notification_plugin())
            .build(context)
            .expect("mock app");
        if let Some(link) = link {
            app.manage(link);
        }
        let window =
            tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(page)))
                .build()
                .expect("main window");
        Till { _app: app, window }
    }

    fn ask_notification(till: &Till, command: &str) -> Result<serde_json::Value, serde_json::Value> {
        let body = match command {
            "register_listener" => serde_json::json!({ "event": "actionPerformed", "handler": 7 }),
            _ => serde_json::json!({}),
        };
        tauri::test::get_ipc_response(
            &till.window,
            tauri::webview::InvokeRequest {
                cmd: format!("plugin:notification|{command}"),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: bundled_page(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<serde_json::Value>().expect("json answer"))
    }

    /// What a computer answers once the subscription got past the gate: the plugin has no Rust
    /// handler for it, and a phone would hand it to its Kotlin/Swift half.
    fn reached_the_plugin() -> Result<serde_json::Value, serde_json::Value> {
        Err(serde_json::Value::from("Command register_listener not found"))
    }

    #[test]
    fn the_website_cannot_hear_the_taps_on_the_notices_of_a_till_linked_to_a_hub() {
        let till = till_with_the_notification_plugin("https://www.erplora.com/", Some(linked_to_the_hub()));
        assert_eq!(ask_notification(&till, "register_listener"), refused());
        assert_eq!(ask_notification(&till, "is_permission_granted"), refused());
    }

    #[test]
    fn another_business_cannot_hear_the_taps_on_the_notices_of_the_till() {
        let till = till_with_the_notification_plugin(
            "https://otronegocio.a.erplora.com/m/sales",
            Some(linked_to_the_hub()),
        );
        assert_eq!(ask_notification(&till, "register_listener"), refused());
    }

    #[test]
    fn the_onboarding_cannot_hear_the_taps_on_the_notices() {
        let till = till_with_the_notification_plugin("https://erplora.com/shell/", Some(on_the_onboarding()));
        assert_eq!(ask_notification(&till, "register_listener"), refused());
    }

    #[test]
    fn the_linked_hub_hears_the_taps_on_its_notices() {
        let till = till_with_the_notification_plugin(
            "https://panaderia.a.erplora.com/m/sales",
            Some(linked_to_the_hub()),
        );
        assert_eq!(ask_notification(&till, "register_listener"), reached_the_plugin());
        assert_eq!(ask_notification(&till, "is_permission_granted"), Ok(serde_json::Value::Bool(true)));
    }

    #[test]
    fn the_hub_cannot_hear_the_taps_while_the_page_that_sent_the_window_there_may_still_run() {
        use tauri::Manager;
        let link = linked_to_the_hub();
        link.follow(&url("https://otronegocio.a.erplora.com/"));
        link.follow(&url("https://panaderia.a.erplora.com/"));
        let till = till_with_the_notification_plugin("https://panaderia.a.erplora.com/", Some(link));
        assert_eq!(ask_notification(&till, "register_listener"), refused(), "the other business still runs");
        till._app.state::<HubLink>().landed(&url("https://panaderia.a.erplora.com/"));
        assert_eq!(ask_notification(&till, "register_listener"), reached_the_plugin());
    }

    #[test]
    fn the_wrapped_notification_plugin_is_still_the_plugin_the_page_talks_to() {
        // The gate wraps the plugin, it does not replace it: the same name routes
        // `plugin:notification|…` to it, and the same script still puts the plugin's
        // `window.Notification` in every page.
        use tauri::plugin::Plugin;
        let raw = tauri_plugin_notification::init::<tauri::test::MockRuntime>();
        let wrapped = notification_plugin::<tauri::test::MockRuntime>();
        assert_eq!(wrapped.name(), raw.name());
        assert!(raw.initialization_script().is_some(), "the plugin ships a script");
        assert_eq!(wrapped.initialization_script(), raw.initialization_script());
    }

    #[test]
    fn without_a_link_state_nobody_hears_the_taps() {
        let till = till_with_the_notification_plugin("https://panaderia.a.erplora.com/", None);
        assert_eq!(ask_notification(&till, "register_listener"), refused());
    }
}
