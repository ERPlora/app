//! «Continue with Google» on Android, in the device's browser.
//!
//! Google's page inside the window sees neither the Google accounts of the tablet nor the browser's
//! session, so it asks for the address and the password from scratch, and Google forbids signing in
//! inside an embedded web view. So on Android the sign-in leaves for the system browser and the
//! session comes back to the window with a one-time code, the way OAuth for native apps does it
//! (PKCE, RFC 7636):
//!
//! 1. The window is about to load Google's page: the app makes a secret of its own and loads the
//!    SaaS's `/auth/app/start/` with its challenge, which the SaaS keeps in the window's session.
//! 2. The start sends the window to `/auth/app/google/`, which opens in a Custom Tab instead: Google's
//!    «Choose an account», with the accounts of the device.
//! 3. Signed in there, the browser offers «Open the ERPlora app», a link `erplora://signed-in?code=…`.
//! 4. The app loads `/auth/app/finish/` with that code and the secret: the SaaS opens the session in
//!    the window. Another app that catches the link has the code without the secret: it opens nothing.
//!
//! The window loads the start and the finish only when the app itself asks for them, once each
//! ([`AppSignIn::admits`]): a link of any page to either is refused (`navigation.rs`).

use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tauri::Url;

pub const START_PATH: &str = "/auth/app/start/";
pub const BROWSER_PATH: &str = "/auth/app/google/";
pub const FINISH_PATH: &str = "/auth/app/finish/";
/// `erplora://signed-in?code=…`, the button of the browser's «You are signed in».
pub const RETURN_HOST: &str = "signed-in";

/// The S256 challenge of `verifier`: BASE64URL, without padding, of its SHA-256.
pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 32 random bytes in BASE64URL: the 43 characters RFC 7636 recommends.
pub fn new_verifier() -> String {
    let mut bytes = [0u8; 32];
    if let Err(e) = getrandom::getrandom(&mut bytes) {
        // Without the system's randomness there is no secret to keep: no sign-in can start.
        log::error!("shell: no randomness for the sign-in secret: {e}");
        return String::new();
    }
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Where the person was going: the `next` of the page they tapped «Continue with Google» on, as the
/// sign-in form uses it, when it is a path of the same site.
pub fn next_of(page: &Url) -> Option<String> {
    let (_, next) = page.query_pairs().find(|(key, _)| key == "next")?;
    let same_site = next.starts_with('/') && !next.starts_with("//") && !next.starts_with("/\\");
    same_site.then(|| next.into_owned())
}

fn saas_page(saas_base: &str, path: &str, query: &[(&str, &str)]) -> Option<Url> {
    let mut page = saas_base.parse::<Url>().ok()?.join(path).ok()?;
    page.query_pairs_mut().extend_pairs(query);
    Some(page)
}

pub fn start_url(saas_base: &str, challenge: &str, next: Option<&str>) -> Option<Url> {
    let mut query = vec![("challenge", challenge)];
    query.extend(next.map(|next| ("next", next)));
    saas_page(saas_base, START_PATH, &query)
}

pub fn finish_url(saas_base: &str, code: &str, verifier: &str) -> Option<Url> {
    saas_page(saas_base, FINISH_PATH, &[("code", code), ("verifier", verifier)])
}

/// The code of the sign-in's way back, `erplora://signed-in?code=…`; any other link is not one.
pub fn returned_code(link: &str) -> Option<String> {
    let link = link.parse::<Url>().ok()?;
    if link.scheme() != crate::DEEP_LINK_SCHEME || link.host_str() != Some(RETURN_HOST) {
        return None;
    }
    let (_, code) = link.query_pairs().find(|(key, _)| key == "code")?;
    (!code.is_empty()).then(|| code.into_owned())
}

/// The sign-in under way: the secret, and the one page the window may load next on its account.
struct Pending {
    verifier: Option<String>,
    expected: Option<Url>,
}

/// The sign-in the app started in the browser, if any. One at a time: a new start forgets the last.
#[derive(Default)]
pub struct AppSignIn(Mutex<Option<Pending>>);

impl AppSignIn {
    /// A new secret, and the start the window loads with its challenge.
    pub fn begin(&self, saas_base: &str, next: Option<&str>) -> Option<Url> {
        let verifier = new_verifier();
        if verifier.is_empty() {
            return None;
        }
        let start = start_url(saas_base, &challenge_of(&verifier), next)?;
        *self.0.lock().ok()? = Some(Pending { verifier: Some(verifier), expected: Some(start.clone()) });
        Some(start)
    }

    /// The finish the window loads with the code the browser handed back, once per start.
    pub fn finish(&self, saas_base: &str, code: &str) -> Option<Url> {
        let mut pending = self.0.lock().ok()?;
        let state = pending.as_mut()?;
        let finish = finish_url(saas_base, code, &state.verifier.take()?)?;
        state.expected = Some(finish.clone());
        Some(finish)
    }

    /// Is `target` the page the app just asked the window for? Yes once: the next load is a link.
    pub fn admits(&self, target: &Url) -> bool {
        let Ok(mut pending) = self.0.lock() else {
            return false;
        };
        match pending.as_mut() {
            Some(state) if state.expected.as_ref() == Some(target) => {
                state.expected = None;
                true
            }
            _ => false,
        }
    }
}

/// Google's page on Android: a new secret, and the window to the start with it (`NavigationVerdict::AppSignIn`).
/// The destination is the one of the page the person tapped «Continue with Google» on.
pub(crate) fn start<R: tauri::Runtime>(app: &tauri::AppHandle<R>, saas_base: &str) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        log::error!("shell: no main window to start the sign-in with Google in");
        return;
    };
    let next = window.url().ok().and_then(|page| next_of(&page));
    let started = app.try_state::<AppSignIn>().and_then(|sign_in| sign_in.begin(saas_base, next.as_deref()));
    let Some(start) = started else {
        log::error!("shell: the sign-in with Google could not start");
        say_the_browser_did_not_open(&window);
        return;
    };
    if let Err(e) = window.navigate(start) {
        log::error!("shell: could not load the start of the sign-in with Google: {e}");
    }
}

/// `erplora://signed-in?code=…`: the window to the finish of the sign-in it started. `false` for any
/// other link; a way back with no sign-in under way (the app was closed meanwhile) is ignored.
pub(crate) fn come_back<R: tauri::Runtime>(app: &tauri::AppHandle<R>, saas_base: &str, link: &str) -> bool {
    use tauri::Manager;
    let Some(code) = returned_code(link) else {
        return false;
    };
    let finish = app.try_state::<AppSignIn>().and_then(|sign_in| sign_in.finish(saas_base, &code));
    let (Some(finish), Some(window)) = (finish, app.get_webview_window("main")) else {
        log::warn!("shell: a way back from the sign-in with Google with no sign-in under way");
        return true;
    };
    if let Err(e) = window.navigate(finish) {
        log::error!("shell: could not load the finish of the sign-in with Google: {e}");
    }
    true
}

/// The browser half of the sign-in, in a tab of the device's browser over the app; in the browser
/// itself when no tab opens (`NavigationVerdict::SignInTab`).
pub(crate) fn open_tab(app: &tauri::AppHandle, target: &Url) {
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;
    if let Err(e) = app.erplora_android().open_custom_tab(target.as_str()) {
        log::warn!("shell: no browser tab for the sign-in with Google ({e}); the browser itself opens it");
        crate::window::open_in_browser(app, target);
    }
}

fn say_the_browser_did_not_open<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Err(e) = window.eval(crate::window::browser_failure_notice_script()) {
        log::error!("shell: could not say that the sign-in with Google did not start: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAAS: &str = "https://erplora.com";

    fn url(raw: &str) -> Url {
        raw.parse().expect("test URL")
    }

    fn query_of(target: &Url, key: &str) -> Option<String> {
        target.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    }

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn every_sign_in_gets_a_secret_of_its_own_in_the_unreserved_alphabet() {
        let (first, second) = (new_verifier(), new_verifier());
        assert_eq!(first.len(), 43, "32 random bytes, base64url without padding");
        assert!(first.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{first}");
        assert_ne!(first, second);
    }

    #[test]
    fn the_destination_is_the_same_site_next_of_the_page_the_person_tapped_on() {
        assert_eq!(
            next_of(&url("https://erplora.com/launch?next=%2Fhub-login%2F%3Fhub%3Da")).as_deref(),
            Some("/hub-login/?hub=a")
        );
        for page in [
            "https://erplora.com/launch",
            "https://erplora.com/launch?next=%2F%2Fevil.example%2F",
            "https://erplora.com/launch?next=https%3A%2F%2Fevil.example%2F",
            "https://erplora.com/launch?next=%2F%5Cevil.example",
        ] {
            assert_eq!(next_of(&url(page)), None, "{page}");
        }
    }

    #[test]
    fn the_start_carries_the_challenge_and_the_destination() {
        let start = start_url(SAAS, "C", Some("/hub-login/?hub=a")).expect("start");
        assert_eq!((start.origin().ascii_serialization(), start.path()), (SAAS.to_string(), START_PATH));
        assert_eq!(query_of(&start, "challenge").as_deref(), Some("C"));
        assert_eq!(query_of(&start, "next").as_deref(), Some("/hub-login/?hub=a"));
        let bare = start_url(SAAS, "C", None).expect("start");
        assert_eq!(query_of(&bare, "next"), None);
    }

    #[test]
    fn the_finish_carries_the_code_and_the_secret() {
        let finish = finish_url(SAAS, "the-code", "the-secret").expect("finish");
        assert_eq!(finish.path(), FINISH_PATH);
        assert_eq!(query_of(&finish, "code").as_deref(), Some("the-code"));
        assert_eq!(query_of(&finish, "verifier").as_deref(), Some("the-secret"));
    }

    #[test]
    fn only_the_way_back_of_the_sign_in_carries_a_code() {
        assert_eq!(returned_code("erplora://signed-in?code=abc").as_deref(), Some("abc"));
        for link in ["erplora://signed-in", "erplora://signed-in?code=", "erplora://hub/x?code=abc", "https://signed-in/?code=abc"] {
            assert_eq!(returned_code(link), None, "{link}");
        }
    }

    #[test]
    fn the_window_loads_the_start_and_the_finish_the_app_asked_for_once() {
        let sign_in = AppSignIn::default();
        let start = sign_in.begin(SAAS, None).expect("start");
        let challenge = query_of(&start, "challenge").expect("challenge");
        assert!(sign_in.admits(&start));
        assert!(!sign_in.admits(&start), "the same start loaded twice");

        let finish = sign_in.finish(SAAS, "the-code").expect("finish");
        let verifier = query_of(&finish, "verifier").expect("verifier");
        assert_eq!(challenge_of(&verifier), challenge, "the finish proves the secret of the start");
        assert!(!sign_in.admits(&url("https://erplora.com/auth/app/finish/?code=other&verifier=x")));
        assert!(sign_in.admits(&finish));
        assert!(!sign_in.admits(&finish), "the same finish loaded twice");
        assert_eq!(sign_in.finish(SAAS, "again"), None, "a second way back without a new start");
    }

    #[test]
    fn a_way_back_with_no_sign_in_started_opens_nothing() {
        assert_eq!(AppSignIn::default().finish(SAAS, "the-code"), None);
    }

    fn window_at(page: &str) -> (tauri::App<tauri::test::MockRuntime>, tauri::WebviewWindow<tauri::test::MockRuntime>) {
        use tauri::Manager;
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        app.manage(AppSignIn::default());
        let window = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(page)))
            .build()
            .expect("mock window");
        (app, window)
    }

    #[test]
    fn google_s_page_takes_the_window_to_the_start_with_the_destination_of_the_page() {
        use tauri::Manager;
        let (app, window) = window_at("https://erplora.com/launch?next=%2Fhub-login%2F%3Fhub%3Da");
        start(app.handle(), SAAS);
        let shown = window.url().expect("url");
        assert_eq!(shown.path(), START_PATH);
        assert_eq!(query_of(&shown, "next").as_deref(), Some("/hub-login/?hub=a"));
        assert!(app.state::<AppSignIn>().admits(&shown), "the window would refuse the start it was sent to");
    }

    #[test]
    fn the_way_back_takes_the_window_to_the_finish_of_the_sign_in_it_started() {
        let (app, window) = window_at("https://erplora.com/launch");
        start(app.handle(), SAAS);
        let challenge = query_of(&window.url().expect("url"), "challenge").expect("challenge");
        assert!(come_back(app.handle(), SAAS, "erplora://signed-in?code=the-code"));
        let shown = window.url().expect("url");
        assert_eq!(shown.path(), FINISH_PATH);
        assert_eq!(query_of(&shown, "code").as_deref(), Some("the-code"));
        assert_eq!(challenge_of(&query_of(&shown, "verifier").expect("verifier")), challenge);
    }

    #[test]
    fn a_hub_link_is_not_the_way_back_and_a_way_back_without_a_start_stays_put() {
        let (app, window) = window_at("https://erplora.com/launch");
        assert!(!come_back(app.handle(), SAAS, "erplora://hub/panaderia.a.erplora.com"));
        assert!(come_back(app.handle(), SAAS, "erplora://signed-in?code=the-code"), "the way back is never a hub link");
        assert_eq!(window.url().expect("url").as_str(), "https://erplora.com/launch");
    }
}
