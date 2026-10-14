//! Guard on the Android window reserving the top system inset (hub#1719).
//!
//! `MainActivity` opts into edge-to-edge, which on `targetSdk = 36` is not really a choice: from
//! Android 15 the framework lays every window out behind the system bars whether the app asks for
//! it or not. Opting in is half the contract; the other half is reserving the inset, and that half
//! was missing. The WebView therefore started at the physical top of the screen, so **any** page
//! loaded inside it — the sign-in page the SaaS serves, the till, a module — painted over the
//! clock. QA reproduced it twice on 2026-09-09 (emulator `Pixel_10_Pro` / Android 16, APK 1.1.21):
//! the back arrow of the sign-in page landed on top of the system clock, and the search layer of
//! `/m/sales/pos` blacked the status bar out entirely.
//!
//! The reserving itself lives in `SystemBarInsets` (plugin module), where the Kotlin unit tests
//! reach it on every PR. What no unit test can reach is the wiring: `MainActivity` belongs to
//! `:app`, and `:app` cannot even be configured without files that `cargo tauri android build`
//! writes, so nothing compiles it until the release job. Dropping the call would therefore be
//! silent — green PR, green tags, and an APK painting under the clock again — which is exactly the
//! shape of regression this file exists to stop.
//!
//! The same goes for the COLOUR of the reserved strip (hub#1903): it is the window background the
//! app theme declares, and it has to be the header colour of the hub shell in light and in dark,
//! or a band of another colour sits between the clock and the header.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // `src-tauri` → up one.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("cannot resolve the repository root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A file with every comment line removed: a `//` line explaining a rule is not the rule.
fn code_of(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !(line.starts_with("//") || line.starts_with("*") || line.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const MAIN_ACTIVITY: &str =
    "src-tauri/gen/android/app/src/main/java/com/erplora/app/MainActivity.kt";
const INSETS_HELPER: &str =
    "crates/tauri-plugin-erplora-android/android/src/main/java/com/erplora/android/SystemBarInsets.kt";

/// The activity must actually reserve the strip.
///
/// This is the whole user-visible promise of hub#1719: open the app on a phone and nothing paints
/// over the system clock. It holds for every page because it is applied to the window, not to a
/// template.
#[test]
fn the_activity_reserves_the_top_system_inset() {
    let activity = code_of(&read(MAIN_ACTIVITY));

    assert!(
        activity.contains("SystemBarInsets.applyTopSystemBarPadding"),
        "{MAIN_ACTIVITY} no longer reserves the top system inset: with edge-to-edge on, the \
         WebView goes back to starting at the physical top of the screen and every page it loads \
         paints over the clock (hub#1719)",
    );
}

/// And it must keep opting into edge-to-edge, so exactly ONE party reserves the strip.
///
/// On API 35+ the framework ignores the opt-out, so dropping the call changes nothing there — the
/// padding still applies and the layout is correct. Below API 35 it is not a no-op: the system
/// would inset the window itself, and the padding would then be added on top of an already-inset
/// window, leaving a double margin on every Android 7-14 device. Keeping the call is what makes
/// the two halves of the contract line up across the whole `minSdk = 24` range.
#[test]
fn the_activity_still_opts_into_edge_to_edge() {
    let activity = code_of(&read(MAIN_ACTIVITY));

    assert!(
        activity.contains("enableEdgeToEdge()"),
        "{MAIN_ACTIVITY} stopped calling enableEdgeToEdge(): below API 35 the system would inset \
         the window on its own and the padding would land on top of it, leaving a double margin \
         (hub#1719)",
    );
}

/// The helper must cover the cutout too, and must not swallow the insets whole.
///
/// Neither is reachable from a JVM unit test — `WindowInsetsCompat` is an `android.jar` stub
/// there — and both fail quietly:
///
/// - asking only for `systemBars()` leaves a notch painting over the content on a cutout device,
///   because the status bar can be hidden while the cutout stays carved into the panel;
/// - returning `CONSUMED` would zero EVERY inset for the WebView below, so the page's own
///   `env(safe-area-inset-*)` rules — the bottom gesture bar, the assistant drawer — would go flat
///   and we would trade a top overlap for a bottom one.
#[test]
fn the_helper_covers_the_cutout_and_keeps_the_other_insets_alive() {
    let helper = code_of(&read(INSETS_HELPER));

    assert!(
        helper.contains("systemBars()"),
        "{INSETS_HELPER} must ask for systemBars(): it is the status bar the clock lives in \
         (hub#1719)",
    );
    assert!(
        helper.contains("displayCutout()"),
        "{INSETS_HELPER} must ask for displayCutout() as well: the status bar can be hidden, the \
         notch cannot, and then nothing would reserve the strip (hub#1719)",
    );
    assert!(
        !helper.contains("CONSUMED"),
        "{INSETS_HELPER} must not consume every inset: that zeroes env(safe-area-inset-bottom) \
         too, and the tabbar goes back under the gesture bar (hub#1719)",
    );
}

/// And the strip it just reserved must be SPENT on the way down, or it gets reserved twice.
///
/// hub#1719 padded the window and handed the insets on untouched, so the WebView still reported
/// the full `env(safe-area-inset-top)` and Ionic added it a second time on the first toolbar
/// (`ion-header ion-toolbar:first-of-type { padding-top: var(--ion-safe-area-top) }`). QA measured
/// the result with CDP on 2026-09-16 over the APK of `fb3b48a`: `screen.height` 952 against
/// `innerHeight` 900 on `Pixel_10_Pro` and 800 against 776 on `Pixel_Tablet` — the window already
/// started 52 / 24 CSS px down — while `env(safe-area-inset-top)` still reported those same 52 / 24.
/// The header floated half a bar below the clock on every screen (hub#1895).
///
/// `WindowInsetsCompat.inset(0, reserved, 0, 0)` subtracts the strip from the top of every inset
/// type — the status bar and the cutout — clamping at zero, and leaves left, right, bottom and the
/// IME exactly as they came. Exactly one party reserves the top, and the bottom still reaches the
/// page.
#[test]
fn the_helper_spends_the_strip_it_reserved_before_handing_the_insets_down() {
    let helper = code_of(&read(INSETS_HELPER));

    assert!(
        helper.contains("insets.inset(0, reserved, 0, 0)"),
        "{INSETS_HELPER} hands the page the insets it already spent on padding: the WebView \
         reports the full env(safe-area-inset-top) and Ionic reserves the status bar a second \
         time, leaving an empty strip under the clock on every screen (hub#1895)",
    );
    assert!(
        helper.contains("topInset(bars.top, cutout.top)"),
        "{INSETS_HELPER} must spend the strip THIS device reports — 52 CSS px on a Pixel 10 Pro, \
         24 on a Pixel Tablet. A fixed number fits one of the two and breaks the other (hub#1895)",
    );
}

// ── The colour of the strip (hub#1903) ──────────────────────────────────────────────────────────

// The comparison against the hub shell's header colour (`apps/web/src/theme/variables.css`) did
// not move with the app: that theme stayed in ERPlora/hub (README.md, «Tests retirados»). What is
// still checked here is the Android half: the theme paints the strip with an opaque colour of its
// own, in light and in dark.
const ANDROID_RES: &str = "src-tauri/gen/android/app/src/main/res";
const APP_THEME: &str = "Theme.erplora_tauri";

/// `#rrggbb` for an opaque `#rgb`-family hex colour (`#rrggbb` or Android's `#aarrggbb` with
/// `ff` alpha); `None` for anything translucent or not a hex colour at all.
fn opaque_rgb(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix('#')?.to_ascii_lowercase();
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => Some(format!("#{hex}")),
        8 if hex.starts_with("ff") => Some(format!("#{}", &hex[2..])),
        _ => None,
    }
}

/// A resource file as Android resolves it for `qualifier`: the qualified directory if it has the
/// file, `values/` otherwise.
fn android_resource(qualifier: &str, file: &str) -> (String, String) {
    let qualified = format!("{ANDROID_RES}/{qualifier}/{file}");
    if repo_root().join(&qualified).exists() {
        let text = read(&qualified);
        return (qualified, text);
    }
    let base = format!("{ANDROID_RES}/values/{file}");
    let text = read(&base);
    (base, text)
}

/// The text between `<tag … name="name" …>` and its closing tag.
fn xml_named<'a>(xml: &'a str, tag: &str, name: &str) -> Option<&'a str> {
    let marker = format!("name=\"{name}\"");
    let mut rest = xml;
    while let Some(open) = rest.find(&format!("<{tag}")) {
        let element = &rest[open..];
        let head_end = element.find('>')?;
        if element[..head_end].contains(&marker) {
            let body = &element[head_end + 1..];
            return Some(&body[..body.find(&format!("</{tag}>"))?]);
        }
        rest = &element[head_end..];
    }
    None
}

/// The colour the strip above the page is painted with on a device in `qualifier`.
///
/// The strip is the top padding of the activity's content view, which is transparent (see
/// `the_strip_is_painted_by_the_window_and_by_nothing_else`), so it shows the window background
/// the app theme declares.
fn strip_colour(qualifier: &str) -> String {
    let (themes_file, themes) = android_resource(qualifier, "themes.xml");
    let style = xml_named(&themes, "style", APP_THEME)
        .unwrap_or_else(|| panic!("{themes_file} does not declare {APP_THEME}"));
    let background = xml_named(style, "item", "android:windowBackground").unwrap_or_else(|| {
        panic!(
            "{themes_file} leaves android:windowBackground to the parent theme: the strip under \
             the clock is then MaterialComponents' own white (#121212 in dark), not the header \
             colour (hub#1903)"
        )
    });
    let colour_name = background.trim().strip_prefix("@color/").unwrap_or_else(|| {
        panic!("{themes_file}: android:windowBackground must be a @color/ resource, got `{background}`")
    });

    let qualified = format!("{ANDROID_RES}/{qualifier}/colors.xml");
    let from_qualified = repo_root()
        .join(&qualified)
        .exists()
        .then(|| read(&qualified))
        .and_then(|xml| xml_named(&xml, "color", colour_name).map(str::to_string));
    let value = from_qualified.unwrap_or_else(|| {
        let base = read(&format!("{ANDROID_RES}/values/colors.xml"));
        xml_named(&base, "color", colour_name)
            .unwrap_or_else(|| panic!("no @color/{colour_name} in {ANDROID_RES}/values/colors.xml"))
            .to_string()
    });
    opaque_rgb(&value).unwrap_or_else(|| {
        panic!(
            "@color/{colour_name} ({qualifier}) is `{value}`: the strip must be an opaque colour"
        )
    })
}

/// In light mode the strip under the clock wears the header colour, so the header reaches the edge.
///
/// Measured by the driver on 2026-09-18 over v1.1.26 on `Pixel_10_Pro` (`adb exec-out screencap`):
/// the strip was `(255, 255, 255)` — MaterialComponents' `DayNight` window background — and the
/// «Inicio» header right below it `(246, 247, 249)`, a white band sitting on a grey bar. Every
/// native app carries the top bar's colour up to the edge and rests the clock on it.
#[test]
fn the_strip_under_the_clock_is_an_opaque_theme_colour_in_light_mode() {
    // `strip_colour` panics unless the theme declares `android:windowBackground` as an opaque
    // `@color/` resource. The `assert_eq!` against the header colour of the web theme is retired.
    strip_colour("values");
}

/// And in dark mode, where the parent theme would paint `#121212` over a `#0b0c0e` header.
#[test]
fn the_strip_under_the_clock_is_an_opaque_theme_colour_in_dark_mode() {
    // Same as the light mode test: the Android half only (README.md, «Tests retirados»).
    strip_colour("values-night");
}

/// Nothing may paint over the window background in that strip.
///
/// The colour above only reaches the screen because the view that carries the padding — the
/// activity's content view — is transparent. Giving it (or any view on the way) a background of its
/// own would put a band of that colour back under the clock, whatever the theme says.
#[test]
fn the_strip_is_painted_by_the_window_and_by_nothing_else() {
    for file in [MAIN_ACTIVITY, INSETS_HELPER] {
        let code = code_of(&read(file));
        for painter in ["setBackground", "background ="] {
            assert!(
                !code.contains(painter),
                "{file} paints a background (`{painter}`) on the way to the WebView: the strip \
                 under the clock stops showing the window background, which is the header colour \
                 (hub#1903)",
            );
        }
    }
}

/// Every resource XML of the app must still be well-formed where a comment is concerned.
///
/// XML forbids `--` inside a comment, and the colours above are named after CSS custom properties
/// that all start with `--`. Quoting one in a resource comment is the natural thing to do and it
/// breaks `aapt2` (`xml parser error: not well-formed`) — but `:app` is only compiled by the release
/// job, so the PR would go green and the tag would come out without an APK.
#[test]
fn the_android_resources_keep_double_hyphens_out_of_their_comments() {
    fn xml_files(dir: &std::path::Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("unreadable resource entry").path();
            if path.is_dir() {
                xml_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "xml") {
                found.push(path);
            }
        }
    }

    let mut files = Vec::new();
    xml_files(&repo_root().join(ANDROID_RES), &mut files);
    assert!(
        !files.is_empty(),
        "no resource XML found under {ANDROID_RES}"
    );

    for path in files {
        let xml = std::fs::read_to_string(&path).expect("unreadable resource XML");
        let mut rest = xml.as_str();
        while let Some(open) = rest.find("<!--") {
            let after = &rest[open + 4..];
            let close = after
                .find("-->")
                .unwrap_or_else(|| panic!("{}: unterminated comment", path.display()));
            assert!(
                !after[..close].contains("--"),
                "{}: `--` inside an XML comment is not well-formed and aapt2 refuses the file; \
                 the release job is the first to compile it (hub#1903)",
                path.display(),
            );
            rest = &after[close + 3..];
        }
    }
}

// `the_colour_tests_run_when_the_header_colour_changes` checked that `test-shell.yml` triggers on
// `apps/web/src/theme/variables.css`; that file is not in this repo (README.md, «Tests retirados»).
