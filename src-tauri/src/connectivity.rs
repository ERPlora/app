//! What the window shows when the NETWORK dies under it (hub#1716).
//!
//! The shell is a thin client (ADR-0159): everything the user sees is served by a remote origin,
//! so a navigation that never lands is not an edge case — in a bar or a salon the line drops every
//! few days. Until hub#1716 the app had no answer for it. The bundled fallback page existed, but
//! `open_main_window` reached it only from the `Err(_)` arm of `initial.parse::<tauri::Url>()`, a
//! branch whose own comment said it should not happen; every real failure landed on whatever the
//! platform paints on a dead load — a **blank white window** on macOS (WKWebView never finishes a
//! failed provisional navigation) and the grey `ERR_INTERNET_DISCONNECTED` page on Android. A
//! Tauri window has no address bar and no reload button, so neither has a way out: the till is
//! dead until somebody kills the app from the operating system.
//!
//! ## Why a probe and not a load-failure callback
//!
//! There isn't one. Tauri 2 offers `on_navigation` (fires BEFORE the load) and `on_page_load`
//! (`Started`/`Finished`), and "finished" is not portable evidence: Android's WebView fires
//! `onPageFinished` for its own error page, so the load looks successful on precisely the platform
//! where the symptom was measured. What IS portable is asking the network ourselves.
//!
//! The rule is deliberately about TRANSPORT, not about HTTP: any response at all — 200, 404, 502 —
//! means the network is up and the page in the window is the origin's own business (a 404 is
//! [`crate::should_forget_hub`]'s problem, not this module's). Only a transport failure (DNS,
//! connect, timeout) means what the user is looking at cannot be us.
//!
//! ## What it costs
//!
//! Nothing while the network is fine. The guard probes once shortly after boot, once per remote
//! navigation, and then sleeps on a [`Notify`] with no timer at all; it only polls — with backoff —
//! while it is showing the offline page, which is exactly when someone is waiting for the network
//! to come back. That matters: this runs on tablets on mobile data.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Runtime, Url, WebviewWindow};
use tokio::sync::Notify;

/// How long a probe waits before calling the target unreachable. Short on purpose: this decides
/// how fast a dead till gets a screen it can act on.
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// Grace before the first probe of a freshly started (or freshly navigated) window. The page is
/// loading over the same network we are about to test; probing in the same instant would race the
/// load for the connection on a slow link.
const FIRST_PROBE_DELAY: Duration = Duration::from_secs(3);

/// Consecutive failed probes before the window is pulled off the target.
///
/// Two, not one: a single transport failure is also what a captive portal, a Wi-Fi roam or a DNS
/// hiccup looks like, and yanking somebody off a page that was working is its own bug. Two failures
/// spaced by [`retry_delay`] is ~5 s — under the time it takes to notice.
const OFFLINE_STRIKES: u32 = 2;

/// Which of the two screens the window is meant to be showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellScreen {
    /// The remote origin the user came for.
    Target,
    /// The bundled page that says the network is down and offers a way back.
    Offline,
}

/// What one probe of the target origin said. See the module note: transport, not HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reachability {
    Online,
    Offline,
}

/// The screen the window must move to, or `None` when it is already on the right one.
///
/// Pure so the policy can be read (and mutated) without a webview: this is the whole decision.
pub fn next_screen(
    current: ShellScreen,
    probe: Reachability,
    consecutive_failures: u32,
) -> Option<ShellScreen> {
    match (current, probe) {
        // Off the target only once the failure has repeated: see `OFFLINE_STRIKES`.
        (ShellScreen::Target, Reachability::Offline) if consecutive_failures >= OFFLINE_STRIKES => {
            Some(ShellScreen::Offline)
        }
        // Back on our own, the moment the network answers. This is the half the issue asks for by
        // name: nobody should have to reload anything.
        (ShellScreen::Offline, Reachability::Online) => Some(ShellScreen::Target),
        _ => None,
    }
}

/// How long to wait before probing again while the offline page is up: 2 s, 4 s, 8 s, 16 s, then
/// every 30 s. Fast enough that a blip is over before the user finishes reading the screen, slow
/// enough that an evening with the router unplugged is not a request per second.
pub fn retry_delay(consecutive_failures: u32) -> Duration {
    const CAP: u64 = 30;
    let seconds = 2u64
        .checked_pow(consecutive_failures.clamp(1, 8))
        .unwrap_or(CAP)
        .min(CAP);
    Duration::from_secs(seconds)
}

/// The URL of the app's OWN bundled page, as Tauri 2 serves the `frontendDist` assets.
///
/// Mirrors `tauri::manager::AppManager::tauri_protocol_url` (tauri 2.11.5), which is `pub(crate)`
/// and therefore cannot be asked at runtime: `wry` cannot register a custom scheme on Windows or
/// Android, so there the assets are served from `http://tauri.localhost` and everywhere else from
/// the `tauri://` scheme. Pinned by the test below; a drift shows up as the offline page not
/// appearing, so [`show`] also refuses to navigate to a URL it could not build.
pub fn bundled_page_url() -> Result<Url, tauri::Error> {
    let raw = if cfg!(windows) || cfg!(target_os = "android") {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    };
    Url::parse(raw).map_err(tauri::Error::InvalidUrl)
}

/// The URL to probe for a target: its ORIGIN, not the page.
///
/// The page may be a deep route that legitimately 404s, and it may be heavy; the origin root
/// answers something as long as there is a network, which is the only question being asked.
pub fn probe_url(target: &Url) -> Url {
    target.join("/").unwrap_or_else(|_| target.clone())
}

/// Whether a URL the webview is navigating to is a REMOTE target worth watching.
///
/// Our own bundled page is not (navigating to it is what this module does), and neither is the
/// platform's error page (`chrome-error://`, `about:`). On Windows and Android the bundled page is
/// served over `http` from `tauri.localhost`, so the scheme alone is not enough to tell them apart.
pub fn remote_target(url: &Url) -> Option<Url> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if url.host_str() == Some("tauri.localhost") {
        return None;
    }
    Some(url.clone())
}

/// Where the window should be, and how the last few probes went.
struct NavState {
    target: Url,
    screen: ShellScreen,
    consecutive_failures: u32,
}

/// Shared between the guard task, the `on_navigation` handler and the `shell_retry` command.
pub struct ShellNav {
    state: Mutex<NavState>,
    /// Raised when something wants a probe NOW: a fresh navigation, or the retry button.
    wake: Notify,
}

impl ShellNav {
    pub fn new(target: Url) -> Self {
        Self {
            state: Mutex::new(NavState {
                target,
                screen: ShellScreen::Target,
                consecutive_failures: 0,
            }),
            wake: Notify::new(),
        }
    }

    /// The origin the user is meant to be on.
    pub fn target(&self) -> Url {
        self.locked().target.clone()
    }

    pub fn screen(&self) -> ShellScreen {
        self.locked().screen
    }

    /// The webview navigated somewhere remote by itself (a link, a redirect, the login flow): that
    /// is the new target, and the slate on it is clean.
    pub fn set_target(&self, target: Url) {
        {
            let mut state = self.locked();
            if state.target == target && state.screen == ShellScreen::Target {
                // Same page again (a reload, a hash change): nothing to reset, but still worth a
                // probe — this is also the path a retry that succeeded comes back through.
                drop(state);
                self.wake.notify_one();
                return;
            }
            state.target = target;
            state.screen = ShellScreen::Target;
            state.consecutive_failures = 0;
        }
        self.wake.notify_one();
    }

    /// Fold one probe into the state and answer with the screen the window must move to.
    fn record(&self, probe: Reachability) -> (Option<ShellScreen>, Url, u32) {
        let mut state = self.locked();
        state.consecutive_failures = match probe {
            Reachability::Online => 0,
            Reachability::Offline => state.consecutive_failures.saturating_add(1),
        };
        let moving = next_screen(state.screen, probe, state.consecutive_failures);
        if let Some(screen) = moving {
            state.screen = screen;
        }
        (moving, state.target.clone(), state.consecutive_failures)
    }

    /// A poisoned lock here would mean a panic inside a two-line critical section; the shell keeps
    /// going with the state as it was left rather than taking the window down with it.
    fn locked(&self) -> std::sync::MutexGuard<'_, NavState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Point the window at `screen`. The one place that navigates, so the mock-runtime test that
/// drives it is testing the real move and not a bool.
pub fn show<R: Runtime>(
    window: &WebviewWindow<R>,
    screen: ShellScreen,
    target: &Url,
) -> tauri::Result<()> {
    let destination = match screen {
        ShellScreen::Target => target.clone(),
        ShellScreen::Offline => bundled_page_url()?,
    };
    window.navigate(destination)
}

/// Ask the network whether `target`'s origin is reachable at all.
async fn probe(target: &Url) -> Reachability {
    let url = probe_url(target);
    // Async reqwest, NOT the blocking client: measured on Android API 37, a blocking request from
    // this crate never left the device (see `spawn_hub_liveness_check`, same trap).
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(client) => client,
        Err(e) => {
            // Nothing the user did, and nothing they can fix — but a shell that cannot build an
            // HTTP client would otherwise report "offline" forever with no trace of why.
            log::error!("shell: could not build the connectivity probe client: {e}");
            return Reachability::Offline;
        }
    };
    match client.head(url.as_str()).send().await {
        // Any answer means the network is up; what the answer SAYS is the origin's business.
        Ok(response) => {
            log::debug!("shell: {url} answered {}", response.status());
            Reachability::Online
        }
        Err(e) => {
            log::info!("shell: {url} is unreachable: {e}");
            Reachability::Offline
        }
    }
}

/// One probe, applied to the window. Shared by the background guard and the retry button so both
/// take the same decision.
pub async fn probe_and_apply<R: Runtime>(window: &WebviewWindow<R>, nav: &ShellNav) -> Reachability {
    let target = nav.target();
    let reachability = probe(&target).await;
    let (moving, target, failures) = nav.record(reachability);

    if let Some(screen) = moving {
        log::info!("shell: network is {reachability:?} after {failures} probe(s); showing {screen:?}");
        if let Err(e) = show(window, screen, &target) {
            // The window stays where it was — blank on macOS, on the error page on Android. Loud,
            // because from the user's seat this is the failure the whole module exists to prevent.
            log::error!("shell: could not move the window to {screen:?}: {e}");
        }
    }
    reachability
}

/// When to probe again after a probe, or `None` to sleep until something asks.
///
/// Pure so the schedule is testable: a guard that stops asking is indistinguishable, from the
/// counter, from no guard at all — which is exactly how the first version of this shipped, keyed
/// on the screen alone, and why the window still came up blank with the guard armed.
pub fn next_probe_delay(screen: ShellScreen, consecutive_failures: u32) -> Option<Duration> {
    match (screen, consecutive_failures) {
        // Still stranded: keep asking, with backoff, so the window comes back by itself.
        (ShellScreen::Offline, failures) => Some(retry_delay(failures)),
        // A strike on the board and the window still on the target: the debounce of
        // [`OFFLINE_STRIKES`] is waiting for a SECOND probe, so somebody has to schedule it. Skip
        // this and the debounce silently means "never".
        (ShellScreen::Target, failures) if failures > 0 => Some(retry_delay(failures)),
        // On the target and reachable: no timer at all until something happens.
        (ShellScreen::Target, _) => None,
    }
}

/// Watch the connection for as long as the window lives.
///
/// Sleeps on [`ShellNav::wake`] while everything is fine, so a healthy till pays nothing for this;
/// polls with [`retry_delay`] only while the offline page is up, which is the half of the issue
/// that asks for the app to come back on its own.
pub fn spawn_connectivity_guard<R: Runtime>(window: WebviewWindow<R>, nav: Arc<ShellNav>) {
    tauri::async_runtime::spawn(async move {
        let mut next_probe_in = Some(FIRST_PROBE_DELAY);
        loop {
            match next_probe_in {
                // A wake during the wait cuts it short — that is the retry button and a fresh
                // navigation both.
                Some(delay) => {
                    let _ = tokio::time::timeout(delay, nav.wake.notified()).await;
                }
                None => nav.wake.notified().await,
            }

            probe_and_apply(&window, &nav).await;

            next_probe_in = next_probe_delay(nav.screen(), failures_of(&nav));
        }
    });
}

fn failures_of(nav: &ShellNav) -> u32 {
    nav.locked().consecutive_failures
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The decision ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn one_failed_probe_does_not_move_the_window() {
        // A single transport failure is also a Wi-Fi roam or a captive portal waking up. Pulling
        // the user off a page that was working would be a bug of our own making.
        assert_eq!(
            next_screen(ShellScreen::Target, Reachability::Offline, 1),
            None
        );
    }

    #[test]
    fn a_repeated_failure_takes_the_window_to_the_offline_page() {
        assert_eq!(
            next_screen(ShellScreen::Target, Reachability::Offline, OFFLINE_STRIKES),
            Some(ShellScreen::Offline)
        );
        assert_eq!(
            next_screen(ShellScreen::Target, Reachability::Offline, 9),
            Some(ShellScreen::Offline)
        );
    }

    #[test]
    fn the_window_returns_to_the_target_on_the_first_answer() {
        // No strikes on the way back: the moment the network answers, the user gets their till.
        assert_eq!(
            next_screen(ShellScreen::Offline, Reachability::Online, 0),
            Some(ShellScreen::Target)
        );
    }

    #[test]
    fn a_screen_that_is_already_right_is_left_alone() {
        // Navigating on every probe would reload the till's page every few seconds.
        assert_eq!(
            next_screen(ShellScreen::Target, Reachability::Online, 0),
            None
        );
        assert_eq!(
            next_screen(ShellScreen::Offline, Reachability::Offline, 7),
            None
        );
    }

    #[test]
    fn the_backoff_grows_and_then_stops_growing() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
        assert_eq!(retry_delay(3), Duration::from_secs(8));
        assert_eq!(retry_delay(4), Duration::from_secs(16));
        // A router left unplugged overnight must not become a request per second, and must not
        // overflow into a wait nobody ever wakes from either.
        assert_eq!(retry_delay(5), Duration::from_secs(30));
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(30));
        // Zero can only reach here through a caller bug; it must still be a wait, not a spin.
        assert_eq!(retry_delay(0), Duration::from_secs(2));
    }

    #[test]
    fn a_first_failure_still_schedules_the_probe_that_would_confirm_it() {
        // MEASURED on the real app (hub#1716): with the schedule keyed on the SCREEN alone, the
        // first failed probe left the window on `Target`, so the guard went back to sleep on the
        // `Notify` with no timer — and the second strike, the one `OFFLINE_STRIKES` is waiting
        // for, never came. The window stayed blank for as long as anyone watched it. The debounce
        // has to cost a wait, not the whole feature.
        assert_eq!(
            next_probe_delay(ShellScreen::Target, 1),
            Some(retry_delay(1)),
            "a strike on the board with no probe scheduled means the debounce never resolves"
        );
    }

    #[test]
    fn a_clean_target_costs_no_timer_at_all() {
        // The other half: a healthy till must not poll. This runs on tablets on mobile data.
        assert_eq!(next_probe_delay(ShellScreen::Target, 0), None);
    }

    #[test]
    fn the_offline_page_keeps_asking_with_backoff() {
        assert_eq!(next_probe_delay(ShellScreen::Offline, 2), Some(retry_delay(2)));
        assert_eq!(next_probe_delay(ShellScreen::Offline, 9), Some(retry_delay(9)));
    }

    // ── What gets probed, and what counts as a target ────────────────────────────────────────────

    #[test]
    fn the_probe_asks_the_origin_and_not_the_page() {
        let target = Url::parse("https://panaderia.a.erplora.com/pos/tickets?shell=1").unwrap();
        assert_eq!(
            probe_url(&target).as_str(),
            "https://panaderia.a.erplora.com/"
        );
        // A dev hub on loopback keeps its port, or the probe would ask a server that is not there.
        let dev = Url::parse("http://127.0.0.1:8787/some/route").unwrap();
        assert_eq!(probe_url(&dev).as_str(), "http://127.0.0.1:8787/");
    }

    #[test]
    fn our_own_pages_are_never_taken_for_a_target() {
        // The guard navigates to these; taking one for the target would make the offline page
        // probe ITSELF and never come back.
        assert_eq!(remote_target(&Url::parse("tauri://localhost/index.html").unwrap()), None);
        assert_eq!(
            remote_target(&Url::parse("http://tauri.localhost/index.html").unwrap()),
            None
        );
        assert_eq!(
            remote_target(&Url::parse("https://tauri.localhost/index.html").unwrap()),
            None
        );
        // Nor is the platform's own error page a place to come back to.
        assert_eq!(remote_target(&Url::parse("about:blank").unwrap()), None);
        assert_eq!(
            remote_target(&Url::parse("chrome-error://chromewebdata/").unwrap()),
            None
        );
    }

    #[test]
    fn a_hub_and_the_dev_loopback_are_targets() {
        for raw in [
            "https://panaderia.a.erplora.com/?shell=1",
            "https://erplora.com/onboarding",
            "http://127.0.0.1:5173/",
        ] {
            let url = Url::parse(raw).unwrap();
            assert_eq!(
                remote_target(&url).as_ref().map(Url::as_str),
                Some(raw),
                "{raw} should be watched as a target"
            );
        }
    }

    #[test]
    fn the_bundled_page_url_is_the_one_tauri_serves() {
        // Mirrors `AppManager::tauri_protocol_url` (tauri 2.11.5). If tauri ever changes where it
        // serves `frontendDist` from, this is the line that has to change with it — and this is
        // the test that says so instead of the offline page silently never appearing.
        let url = bundled_page_url().expect("the bundled page URL literal must parse");
        if cfg!(windows) || cfg!(target_os = "android") {
            assert_eq!(url.as_str(), "http://tauri.localhost/index.html");
        } else {
            assert_eq!(url.as_str(), "tauri://localhost/index.html");
        }
        // Whatever the platform, the guard must never mistake it for a place to come back to.
        assert_eq!(remote_target(&url), None);
    }

    // ── The state the guard keeps ────────────────────────────────────────────────────────────────

    #[test]
    fn a_fresh_navigation_clears_the_strikes_and_the_offline_screen() {
        let nav = ShellNav::new(Url::parse("https://a.erplora.com/").unwrap());
        assert_eq!(nav.record(Reachability::Offline).0, None);
        assert_eq!(nav.record(Reachability::Offline).0, Some(ShellScreen::Offline));
        assert_eq!(nav.screen(), ShellScreen::Offline);

        nav.set_target(Url::parse("https://b.erplora.com/?shell=1").unwrap());
        assert_eq!(nav.screen(), ShellScreen::Target);
        assert_eq!(nav.target().as_str(), "https://b.erplora.com/?shell=1");
        // …and the two strikes are gone, so the new page gets its own chance.
        assert_eq!(nav.record(Reachability::Offline).0, None);
    }

    #[test]
    fn one_answer_wipes_the_strikes() {
        let nav = ShellNav::new(Url::parse("https://a.erplora.com/").unwrap());
        nav.record(Reachability::Offline);
        assert_eq!(nav.record(Reachability::Online).2, 0);
        // So a blip needs two NEW failures to strand the window again.
        assert_eq!(nav.record(Reachability::Offline).0, None);
    }

    // ── …and the window actually moves ───────────────────────────────────────────────────────────

    /// A real `WebviewWindow` on Tauri's mock runtime, which records `navigate()` and answers
    /// `url()` with it. This is what makes the guard's decision testable as a MOVE.
    fn mock_window() -> WebviewWindow<tauri::test::MockRuntime> {
        // `mock_context(noop_assets())` and NOT `generate_context!()`: that macro embeds the
        // Info.plist and the crate already expands it once in `run()`, so a second expansion is a
        // duplicate `_EMBED_INFO_PLIST` symbol and the test crate does not link. Nothing here
        // serves an asset anyway — the assertion is on where the window POINTS.
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        tauri::WebviewWindowBuilder::new(
            &app,
            "main",
            tauri::WebviewUrl::External(Url::parse("https://panaderia.a.erplora.com/?shell=1").unwrap()),
        )
        .build()
        .expect("mock window")
    }

    // ── …and the LOOP actually drives it ─────────────────────────────────────────────────────────

    /// Answers anything on `port` with a bare 200 for as long as it is alive: the line into the
    /// bar, coming back. Dropping it stops the thread.
    struct Origin {
        stop: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for Origin {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn serve(port: u16) -> Origin {
        use std::sync::atomic::Ordering;

        let listener =
            std::net::TcpListener::bind(("127.0.0.1", port)).expect("the origin must come back up");
        listener.set_nonblocking(true).expect("nonblocking listener");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        use std::io::{Read, Write};
                        // Read the request before answering: closing on unread bytes is an RST,
                        // which reqwest reports as a transport failure — the very thing under test.
                        let _ = socket.set_nonblocking(false);
                        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = socket.read(&mut [0u8; 1024]);
                        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
                        let _ = socket.flush();
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        Origin { stop }
    }

    /// Where the window points, polled until it is `expected` or the deadline runs out.
    fn wait_for_window<R: Runtime>(window: &WebviewWindow<R>, expected: &Url, whose_fault: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if window.url().map(|at| at == *expected).unwrap_or(false) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the window is still on {:?} instead of {expected}: {whose_fault}",
            window.url().map(|at| at.to_string())
        );
    }

    #[test]
    fn the_guard_takes_the_window_off_a_dead_origin_by_itself_and_brings_it_back_by_itself() {
        // Every assertion above is on a pure function, and pure functions were ALL GREEN through
        // the version of this guard that shipped broken (hub#1716): `next_screen` was right,
        // `next_probe_delay` was right, and the app still came up blank because the loop asked
        // them the wrong question and then slept forever. The loop is the seam that rots silently,
        // so it is the seam that has to be driven — against a real socket, because "the network
        // came back and nobody reloaded anything" cannot be asserted against a mock.
        //
        // A port we own: with nothing listening every connect is REFUSED instantly, so the network
        // being down costs no `PROBE_TIMEOUT`; binding it later is the line coming back.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port();
        let target = Url::parse(&format!("http://127.0.0.1:{port}/pos/tickets?shell=1"))
            .expect("target url");

        let window = mock_window();
        let nav = Arc::new(ShellNav::new(target.clone()));
        spawn_connectivity_guard(window.clone(), nav.clone());

        // The line is down. Nobody touches anything: the guard has to notice on its own.
        wait_for_window(
            &window,
            &bundled_page_url().expect("bundled page url"),
            "the guard never moved the window to ERPlora's own offline page, so the till is \
             looking at the platform's error screen — blank on macOS, grey on Android — with no \
             way out (hub#1716)",
        );

        // The line is back. Still nobody touches anything.
        let _origin = serve(port);
        wait_for_window(
            &window,
            &target,
            "the window never returned to the hub once the network answered again: somebody has \
             to reload an app that has no reload button (hub#1716)",
        );
        assert_eq!(nav.screen(), ShellScreen::Target);
    }

    #[test]
    fn a_dead_target_leaves_the_window_on_the_bundled_page_and_a_live_one_brings_it_back() {
        // The regression in one test: this is the symptom of hub#1716 (window stuck on the
        // platform's error page) and the fix (our page, then back on its own) as a pair.
        let window = mock_window();
        let target = Url::parse("https://panaderia.a.erplora.com/?shell=1").unwrap();
        let nav = ShellNav::new(target.clone());

        // One failure is not enough to move anybody.
        let (moving, _target_now, _) = nav.record(Reachability::Offline);
        assert_eq!(moving, None);
        assert_eq!(window.url().unwrap().as_str(), target.as_str());

        // The second one is.
        let (moving, target_now, _) = nav.record(Reachability::Offline);
        show(&window, moving.expect("the window must leave a dead target"), &target_now)
            .expect("navigate");
        assert_eq!(
            window.url().unwrap().as_str(),
            bundled_page_url().expect("bundled page url").as_str(),
            "the window did not end up on ERPlora's own offline page: this is exactly the blank \
             macOS window / grey Android error page of hub#1716"
        );

        // Network back: nobody reloads anything.
        let (moving, target_now, _) = nav.record(Reachability::Online);
        show(&window, moving.expect("the window must return to the target"), &target_now)
            .expect("navigate");
        assert_eq!(
            window.url().unwrap().as_str(),
            target.as_str(),
            "the window never came back to the hub on its own (hub#1716)"
        );
    }
}
