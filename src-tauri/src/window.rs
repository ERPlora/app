//! The main window: where it boots, what moves it to a hub, and what it says about a page it
//! refuses.

use std::path::PathBuf;
use std::sync::Arc;

use crate::connectivity::{self, ShellNav, spawn_connectivity_guard};
use crate::device::distribution_channel;
use crate::hub_url::{
    boot_link, clear_hub_url, cold_start_link, forget_destination, initial_url_for, load_hub_url,
    normalize_base, onboarding_url, persist_hub_url, saas_base_url, should_forget_hub,
    trusted_hub_origin, ColdStartLink, HubProbe,
};
use crate::links::deep_link_from_args;
use crate::navigation::{navigation_verdict, NavigationVerdict};
use crate::{hub_link, link_consent, ShellError, DEFAULT_SAAS_URL, ENV_SHELL_URL};

/// Forgets the captured hub and sends the window back to the SaaS onboarding. Two callers: the
/// frontend on a 410 `hub_not_found` (no `choose` → the plain onboarding) and the topbar's «switch
/// business» control (`choose: true` → `?choose=1`, the SaaS hub picker).
/// KEEPS `device.id`, the stable anchor of this installation.
/// The navigation is best effort (no window is not an error).
#[tauri::command]
pub(crate) fn forget_hub(app: tauri::AppHandle, choose: Option<bool>) -> Result<(), ShellError> {
    use tauri::Manager;
    let cache_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| ShellError::Io(e.to_string()))?;
    clear_hub_url(&cache_dir);
    forget_linked_hub(&app);
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(url) =
            forget_destination(&saas_base_url(), choose.unwrap_or(false)).parse::<tauri::Url>()
        {
            let _ = window.navigate(url);
        }
    }
    Ok(())
}

/// Asks in the background whether the remembered hub still exists and, if it does not, forgets it
/// and sends the window back to the onboarding.
///
/// In the background on purpose: the window is already open and showing the hub, so in the normal
/// case (the hub exists) this goes unnoticed. In the bad case the user sees the 404 for an instant
/// and ends up on the onboarding, which is where they can get out from. Blocking the start to
/// avoid that flicker would penalize **every** start for a rare case.
///
/// A `HEAD` is enough and does not download the whole PWA. The timeout can be short: if the hub
/// does not answer in time it is kept, which is the safe decision ([`should_forget_hub`]).
///
/// It runs on Tauri's **async** runtime with reqwest's async client, NOT on `std::thread` +
/// `reqwest::blocking`. Measured on the API 37 emulator: with the blocking client the request
/// **never left** the device on Android (not a single `HEAD` reached the server), so the rescue did
/// not exist on exactly the platform where the user cannot clear the app's data by hand. On macOS
/// it did work, which is what made it easy to take for granted.
fn spawn_hub_liveness_check(app: tauri::AppHandle, cache_dir: PathBuf, origin: String) {
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;

        let url = format!("{}/", normalize_base(&origin));
        let probe = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
        {
            Ok(c) => match c.head(&url).send().await {
                Ok(r) => HubProbe::Status(r.status().as_u16()),
                Err(e) => {
                    log::warn!("shell: could not reach the remembered hub ({origin}): {e}");
                    HubProbe::Unreachable
                }
            },
            Err(e) => {
                log::warn!("shell: could not build the HTTP client: {e}");
                HubProbe::Unreachable
            }
        };
        if !should_forget_hub(probe) {
            return;
        }

        log::info!("shell: the remembered hub ({origin}) is gone ({probe:?}); back to the onboarding");
        clear_hub_url(&cache_dir);
        forget_linked_hub(&app);
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(url) = onboarding_url(&saas_base_url()).parse::<tauri::Url>() {
                let _ = window.navigate(url);
            }
        }
    });
}

/// The device stops answering the hub it was linked to.
fn forget_linked_hub<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    if let Some(link) = app.try_state::<hub_link::HubLink>() {
        link.forget();
    }
}

/// A link the system handed the app.
///
/// A link to the linked hub just takes the window there. A link to any other hub would hand it the
/// printer, the drawer and the card reader and make it the business the app boots on, and anyone
/// can send one — an email, a web page, a message — so the person in front of the device is asked
/// first, by the system's own dialog: «Open» links it and goes there, anything else leaves the
/// device as it was. With nobody to ask, the link is refused, never followed.
///
/// `target` comes out of [`resolve_deep_link`], already a hub of ours; it is checked again here so
/// this function does not lean on its caller for what it links.
///
/// [`resolve_deep_link`]: crate::resolve_deep_link
pub(crate) fn open_hub_from_link<R: tauri::Runtime>(app: &tauri::AppHandle<R>, target: &str) {
    use tauri::Manager;
    let Some(hub) = target.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin) else {
        log::warn!("shell: link ignored, it does not lead to a hub of ours ({target})");
        return;
    };
    let Some(link) = app.try_state::<hub_link::HubLink>() else {
        log::warn!("shell: link ignored, the device does not know its hub yet ({target})");
        return;
    };
    let linked = link.linked();
    if linked.as_deref() == Some(hub.as_str()) {
        navigate_main_window(app, target);
        return;
    }
    let Some(asker) = app.try_state::<link_consent::AskBeforeLinking>() else {
        log::warn!("shell: link ignored, nobody to ask before linking {hub}");
        return;
    };
    let question = link_consent::Question {
        to: link_consent::host_of(&hub),
        from: linked.as_deref().map(link_consent::host_of),
    };
    let handle = app.clone();
    let target = target.to_string();
    asker.ask(
        question,
        Box::new(move |open| {
            if !open {
                log::info!("shell: the person kept this device on its business; {hub} was not linked");
                return;
            }
            if let Some(link) = handle.try_state::<hub_link::HubLink>() {
                link.link(&hub);
            }
            navigate_main_window(&handle, &target);
        }),
    );
}

/// Takes the main window to `url`. Best effort on purpose: without a window (or with a URL that
/// does not parse) there is nothing to steer, and a link must not bring the app down.
fn navigate_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>, url: &str) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("shell: a link arrived but there is no window to steer yet ({url})");
        return;
    };
    match url.parse::<tauri::Url>() {
        Ok(parsed) => {
            if let Err(e) = window.navigate(parsed) {
                log::warn!("shell: could not open the link ({url}): {e}");
            }
        }
        Err(e) => log::warn!("shell: unreadable link destination ({url}): {e}"),
    }
}

/// The retry button of the offline page.
///
/// A plain `location.reload()` on the bundled fallback page would reload THE FALLBACK PAGE: the
/// only control on the only screen the user can reach would lead back to itself.
/// Retry has to happen on this side, because this is the side that can ask the network and move
/// the window. Answers whether the target is reachable so the page can say "still nothing" instead
/// of pretending it did something.
///
/// Granted to the bundled page ONLY (`capabilities/degraded.json`, `local: true`): a command that
/// navigates the main window is not something a remote origin should be able to call.
#[tauri::command]
pub(crate) async fn shell_retry(
    window: tauri::WebviewWindow,
    nav: tauri::State<'_, Arc<ShellNav>>,
) -> Result<bool, ShellError> {
    let reachability = connectivity::probe_and_apply(&window, nav.inner()).await;
    Ok(reachability == connectivity::Reachability::Online)
}

/// Creates the main window on [`initial_url_for`] and registers the `on_navigation` handler that
/// captures `?shell=1` and persists the origin as `hub.url` (a single write per change).
pub(crate) fn open_main_window(app: &tauri::App, cache_dir: Option<PathBuf>) -> tauri::Result<()> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    let persisted = cache_dir.as_deref().and_then(load_hub_url);
    let override_url = std::env::var(ENV_SHELL_URL)
        .ok()
        .filter(|v| !v.trim().is_empty());
    // COLD start by deep link: on Windows and Linux the system answers a link by launching the
    // executable again with the URL as an argument, so this is where it arrives. On macOS, iOS and
    // Android it arrives as an event and `on_open_url` picks it up (see `run`).
    let deep_link = deep_link_from_args(std::env::args());
    // The second gate in front of the commands: only the linked hub drives the device.
    let (entries, linked) = boot_link(
        override_url.as_deref(),
        persisted.as_deref(),
        &saas_base_url(),
    );
    // Only a link to the linked hub boots on it; one to another hub asks first.
    let cold = cold_start_link(deep_link.as_deref(), linked.as_deref());
    let opens = match &cold {
        ColdStartLink::Open(target) => Some(target.clone()),
        ColdStartLink::None | ColdStartLink::Ask(_) => None,
    };
    let initial = initial_url_for(
        opens.as_deref(),
        override_url.as_deref(),
        persisted.as_deref(),
        &saas_base_url(),
    );
    let url = match initial.parse::<tauri::Url>() {
        Ok(u) => WebviewUrl::External(u),
        Err(e) => {
            // Degraded: the bundled static page (shell-dist). It should not happen except with a
            // malformed ERPLORA_SHELL_URL/ERPLORA_SAAS_URL.
            eprintln!("shell: invalid start URL ({initial}): {e}; loading the degraded page");
            WebviewUrl::App("index.html".into())
        }
    };

    // When the window boots on a remembered hub, it has to be checked that the hub still exists —
    // but AFTER opening the window, not before: blocking a till's start on a network request would
    // be worse than the screen this tries to avoid.
    //
    // Not when the window boots on a link's hub: this check ends up navigating to the onboarding,
    // and would take away exactly what the person just asked for. A link to ANOTHER hub boots on the
    // remembered one, so that one is checked as usual while the person is asked.
    if override_url.is_none() && opens.is_none() {
        if let (Some(dir), Some(origin)) = (cache_dir.clone(), persisted.clone()) {
            spawn_hub_liveness_check(app.handle().clone(), dir, origin);
        }
    }

    // Where the window is MEANT to be, so the connectivity guard can put it back there. Seeded
    // with the URL the window is about to load; `on_navigation` keeps it current afterwards.
    let initial_target = match &url {
        WebviewUrl::External(u) => u.clone(),
        // The degraded arm above: the env URL is unusable, so the address to come back to is the
        // one the app is meant to boot at. Baked literal, not `saas_base_url()`, because what put
        // us in this arm is precisely an env var that does not parse.
        _ => DEFAULT_SAAS_URL
            .parse()
            .map_err(tauri::Error::InvalidUrl)?,
    };
    let hub_link = hub_link::HubLink::new(entries, Some(&initial_target));
    if let Some(origin) = linked {
        hub_link.link(&origin);
    }
    app.manage(hub_link);
    let nav_state = Arc::new(ShellNav::new(initial_target));
    app.manage(nav_state.clone());

    let last = std::sync::Mutex::new(persisted);
    let watched = nav_state.clone();
    let saas_base = saas_base_url();
    let refusals = app.handle().clone();
    let window = WebviewWindowBuilder::new(app, "main", url)
        .title("ERPlora")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 600.0)
        // The hardware waits for the page to have LOADED: the window's URL flips at the
        // start of a navigation, while the page that started it still runs until the new one commits.
        .on_page_load(|window, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                if let Some(link) = window.try_state::<hub_link::HubLink>() {
                    link.landed(payload.url());
                }
            }
        })
        .on_navigation(move |nav| {
            // The Play copy follows only the SaaS pages that cannot take money. First,
            // so a refused page is neither remembered as the hub nor watched by the guard below.
            let verdict = navigation_verdict(distribution_channel(), &saas_base, nav);
            if verdict != NavigationVerdict::Allow {
                refuse_navigation(&refusals, verdict, &saas_base, nav);
                return false;
            }
            // `?shell=1` links a hub only when the SaaS chose it, or it is the linked one.
            let captured = refusals.try_state::<hub_link::HubLink>().and_then(|link| link.follow(nav));
            if let (Some(dir), Some(origin)) = (cache_dir.as_deref(), captured) {
                if let Ok(mut guard) = last.lock() {
                    if guard.as_deref() != Some(origin.as_str()) {
                        match persist_hub_url(dir, &origin) {
                            Ok(()) => *guard = Some(origin),
                            Err(e) => eprintln!("shell: could not persist hub.url: {e}"),
                        }
                    }
                }
            }
            // Wherever the webview goes on its own — a link, a redirect, the login chain — that is
            // the page the guard has to keep alive. Our own bundled page is filtered
            // out inside `remote_target`, or the offline screen would end up watching itself.
            if let Some(target) = connectivity::remote_target(nav) {
                watched.set_target(target);
            }
            true
        })
        .build()?;

    // From here on, a load that never lands has an answer.
    spawn_connectivity_guard(window, nav_state);
    // With the window up, a link to another hub gets its question.
    match cold {
        ColdStartLink::Ask(target) => open_hub_from_link(app.handle(), &target),
        ColdStartLink::None | ColdStartLink::Open(_) => {}
    }
    Ok(())
}

/// The notice for a page the Play copy refused: English source plus its `es` translation. It names
/// no other place to go on purpose — sending the person to finish on the website is the very
/// communication Google Play forbids.
const REFUSAL_NOTICE_EN: &str = "This page is not available in the app.";
const REFUSAL_NOTICE_ES: &str = "Esta página no está disponible en la aplicación.";

/// The script that shows the refusal notice on the page the window stayed on.
///
/// The refused page is the SaaS's, not ours, so there is no i18n runtime to lean on: the language is
/// picked the way the bundled offline page picks it — Spanish unless the device says otherwise. An
/// `alert` because it is the one dialog every webview already paints natively (wry's Android client
/// answers it with an `AlertDialog`). The texts reach the page as JSON string literals, never as
/// code.
fn refusal_notice_script() -> String {
    let es = serde_json::Value::from(REFUSAL_NOTICE_ES);
    let en = serde_json::Value::from(REFUSAL_NOTICE_EN);
    format!(
        "alert(String(navigator.language || \"es\").toLowerCase().indexOf(\"es\") === 0 ? {es} : {en});"
    )
}

/// What the window does with a page `on_navigation` refused: the SaaS home page takes it to the
/// app's own start; any other page leaves it where it was, with the notice.
fn answer_refusal<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    verdict: NavigationVerdict,
    saas_base: &str,
) -> tauri::Result<()> {
    match verdict {
        NavigationVerdict::Allow => Ok(()),
        NavigationVerdict::Home => {
            let start = onboarding_url(saas_base)
                .parse::<tauri::Url>()
                .map_err(tauri::Error::InvalidUrl)?;
            window.navigate(start)
        }
        NavigationVerdict::Refuse => window.eval(refusal_notice_script()),
    }
}

/// Records a refused page and answers it on the main window.
///
/// The answer is sent OFF the handler: on Android `on_navigation` runs inside the webview client's
/// `shouldOverrideUrlLoading`, on the UI thread and under wry's own lock, and it has to return
/// before the webview can do anything else — including what we are about to ask of it.
fn refuse_navigation(
    app: &tauri::AppHandle,
    verdict: NavigationVerdict,
    saas_base: &str,
    target: &tauri::Url,
) {
    use tauri::Manager;

    // Origin and path, never the query: it can carry a one-time code.
    log::warn!(
        "shell: the {} copy refused {}{} ({verdict:?})",
        distribution_channel(),
        target.origin().ascii_serialization(),
        target.path()
    );
    let app = app.clone();
    let saas_base = saas_base.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(window) = app.get_webview_window("main") else {
            log::error!("shell: no main window to answer a refused page on");
            return;
        };
        if let Err(e) = answer_refusal(&window, verdict, &saas_base) {
            log::error!("shell: could not answer a refused page ({verdict:?}): {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{app_with_kept_tap, url};
    #[cfg(desktop)]
    use crate::on_second_launch;

    // ── A link to another hub waits for the person's word ───────────────────────────────────────

    /// A till as it stands after booting on the bakery it remembers (`boot_link` links it, the
    /// window shows it), and the person in front of it: `None` is nobody there to ask.
    fn till_of_the_bakery(
        linked: Option<&str>,
        person: Option<bool>,
    ) -> (tauri::App<tauri::test::MockRuntime>, Arc<std::sync::Mutex<Vec<link_consent::Question>>>) {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let (entries, _) = boot_link(None, linked, "https://erplora.com");
        let showing = linked.map(|hub| url(&format!("{hub}/")));
        let link = hub_link::HubLink::new(entries, showing.as_ref());
        if let Some(hub) = linked {
            link.link(hub);
        }
        app.manage(link);
        let start = showing.unwrap_or_else(|| url("https://erplora.com/shell/"));
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(start))
            .build()
            .expect("mock window");
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        if let Some(open) = person {
            let heard = asked.clone();
            app.manage(link_consent::AskBeforeLinking::new(move |question, answer| {
                heard.lock().expect("questions").push(question);
                answer(open);
            }));
        }
        (app, asked)
    }

    fn window_url(app: &tauri::App<tauri::test::MockRuntime>) -> String {
        use tauri::Manager;
        app.get_webview_window("main").expect("window").url().expect("url").to_string()
    }

    fn linked_hub(app: &tauri::App<tauri::test::MockRuntime>) -> Option<String> {
        use tauri::Manager;
        app.state::<hub_link::HubLink>().linked()
    }

    const BAKERY: &str = "https://panaderia.a.erplora.com";
    const OTHER_LINK: &str = "https://otronegocio.a.erplora.com/?shell=1";

    #[test]
    fn a_link_to_another_hub_does_not_move_the_till_with_nobody_to_ask() {
        let (app, _) = till_of_the_bakery(Some(BAKERY), None);
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY), "the printer went to the link's hub");
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/");
    }

    #[test]
    fn a_link_to_another_hub_links_it_once_the_person_says_open() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(true));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(
            *asked.lock().expect("questions"),
            vec![link_consent::Question {
                to: "otronegocio.a.erplora.com".into(),
                from: Some("panaderia.a.erplora.com".into()),
            }]
        );
        assert_eq!(linked_hub(&app).as_deref(), Some("https://otronegocio.a.erplora.com"));
        assert_eq!(window_url(&app), OTHER_LINK);
    }

    #[test]
    fn cancelling_leaves_the_till_on_its_business() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(asked.lock().expect("questions").len(), 1, "nobody was asked");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/");
    }

    #[test]
    fn a_link_to_the_linked_hub_opens_it_without_asking() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        open_hub_from_link(app.handle(), "https://panaderia.a.erplora.com/?shell=1");
        assert!(asked.lock().expect("questions").is_empty(), "the till asked to open its own hub");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/?shell=1");
    }

    #[test]
    fn a_link_on_a_till_linked_to_nothing_asks_too() {
        // A fresh install (or one that just changed business): yes would link the hub AND remember
        // it for every boot to come, so a link from an email has to ask here as well.
        let (app, asked) = till_of_the_bakery(None, Some(false));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(
            *asked.lock().expect("questions"),
            vec![link_consent::Question { to: "otronegocio.a.erplora.com".into(), from: None }]
        );
        assert_eq!(linked_hub(&app), None);
        assert_eq!(window_url(&app), "https://erplora.com/shell/");
    }

    #[cfg(desktop)]
    #[test]
    fn a_link_handed_over_by_a_second_launch_asks_before_linking_its_hub() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        on_second_launch(app.handle(), vec!["ERPlora.exe".into(), "erplora://hub/otronegocio.a.erplora.com".into()]);
        assert_eq!(asked.lock().expect("questions").len(), 1, "nobody was asked");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
    }

    #[test]
    fn forgetting_the_hub_unlinks_it() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let link = hub_link::HubLink::new(vec!["https://erplora.com".into()], None);
        link.link("https://panaderia.a.erplora.com");
        app.manage(link);
        forget_linked_hub(app.handle());
        assert_eq!(app.state::<hub_link::HubLink>().linked(), None);
    }

    // ── What the window does with a page the Play copy refuses ──────────────────────────────────

    /// A real window on Tauri's mock runtime, which records `navigate()` and answers `url()` with
    /// it — so the answer to a refusal is asserted as a MOVE (or its absence), not as a bool.
    fn mock_window_at(start: &str) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
        // `mock_context(noop_assets())`, not `generate_context!()`: see `connectivity::tests`.
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(start)))
            .build()
            .expect("mock window")
    }

    #[test]
    fn the_saas_home_page_takes_the_window_to_the_app_s_start() {
        let window = mock_window_at("https://erplora.com/accounts/logout/");
        answer_refusal(&window, NavigationVerdict::Home, "https://erplora.com").expect("answer");
        assert_eq!(
            window.url().expect("url").as_str(),
            "https://erplora.com/shell/"
        );
    }

    #[test]
    fn a_refused_page_leaves_the_window_where_it_was() {
        // Anywhere but the app's start, or "stayed" and "went home" would look the same.
        let here = "https://erplora.com/account/login/?next=/shell/";
        let window = mock_window_at(here);
        answer_refusal(&window, NavigationVerdict::Refuse, "https://erplora.com").expect("answer");
        assert_eq!(window.url().expect("url").as_str(), here);
    }

    #[test]
    fn the_refusal_notice_speaks_both_languages_and_names_no_other_place_to_go() {
        let script = refusal_notice_script();
        // The English source and its Spanish translation, picked the way the
        // bundled offline page picks them (Spanish unless the device says otherwise).
        assert!(script.contains(REFUSAL_NOTICE_EN), "{script}");
        assert!(script.contains(REFUSAL_NOTICE_ES), "{script}");
        assert!(script.contains("navigator.language"), "{script}");
        assert!(script.starts_with("alert("), "{script}");
        // Anti-steering cuts both ways: telling the person to finish this on the website is the
        // very communication Play forbids, so the notice may not name any address.
        for text in [REFUSAL_NOTICE_EN, REFUSAL_NOTICE_ES] {
            let lower = text.to_lowercase();
            for forbidden in ["erplora.com", "http", "www", "browser", "navegador", "web"] {
                assert!(
                    !lower.contains(forbidden),
                    "the notice points elsewhere: {text}"
                );
            }
        }
    }
}
