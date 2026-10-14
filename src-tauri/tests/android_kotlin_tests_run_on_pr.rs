//! Guard on WHERE the plugin's Kotlin unit tests run, and on the standalone project that lets
//! them (hub#933).
//!
//! The Kotlin side of the Android plugin — `PermissionPolicy`, `statusOf`, `requestScope`,
//! `BluetoothSpp` — is where the decisions live, and it has 34 unit tests. Until hub#933 the only
//! place they ran was the `build-android` job of `tauri-release.yml`, which fires on a `v*` tag or
//! by hand. So the tests guarded nothing on a PR: `48f2b0e1` (Bluetooth SPP, hub#388) added a
//! permission to the policy, left one expectation stale, and `develop` stayed red for a day
//! without a single check going amber. The next tag would have found it — in the middle of a
//! release, which is the worst possible place to learn about a one-line test expectation.
//!
//! They ran only there for a concrete reason, not by neglect: the Gradle project under
//! `gen/android` cannot be configured without `tauri.settings.gradle` and `app/tauri.build.gradle.kts`,
//! and BOTH are written by `cargo tauri android build`. Running the tests therefore meant paying
//! for the whole Android release build (NDK, four Rust target triples, ~60 min) to reach a step
//! that takes 12 seconds.
//!
//! `crates/tauri-plugin-erplora-android/android-tests/` breaks that coupling: a tiny standalone
//! Gradle root that includes ONLY the plugin library and the `tauri-android` SDK it compiles
//! against, with no `:app` and therefore nothing generated. Measured on this tree: 22 s from cold,
//! 1 s warm, with a JDK and the Android SDK and no NDK or Rust at all.
//!
//! What this file defends is the wiring itself, because both halves fail silently:
//!
//! - a CI step that stops running does not announce it — it just stops being in the log, and the
//!   next stale expectation waits for a tag again;
//! - the standalone project repeats the AGP and Kotlin versions of the generated root, and a
//!   version that drifts does not error either. It compiles the same Kotlin with a different
//!   compiler than the release does, so the check goes green while testing something the shipped
//!   APK is not.
//!
//! Both are cheap to assert from here, and this test already runs on every PR that touches the
//! shell or the plugin (`test-shell.yml`).

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

/// A file with every comment line removed: a `#` or `//` line explaining a rule is not the rule.
fn code_of(source: &str, comment: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with(comment))
        .collect::<Vec<_>>()
        .join("\n")
}

const SHELL_WORKFLOW: &str = ".github/workflows/test-shell.yml";
const STANDALONE_SETTINGS: &str =
    "crates/tauri-plugin-erplora-android/android-tests/settings.gradle.kts";
const STANDALONE_BUILD: &str = "crates/tauri-plugin-erplora-android/android-tests/build.gradle.kts";
const GENERATED_ROOT_BUILD: &str = "src-tauri/gen/android/build.gradle.kts";
const RUNNER_SCRIPT: &str = "scripts/kotlin-plugin-tests.sh";

/// The Kotlin tests must run on a PULL REQUEST, not only when publishing.
///
/// `test-shell.yml` is the right home: it already triggers on `pull_request` filtered to
/// `crates/tauri-plugin-erplora-android/**`, which is exactly the tree these tests cover.
#[test]
fn the_kotlin_unit_tests_run_in_the_pull_request_workflow() {
    let workflow = code_of(&read(SHELL_WORKFLOW), "#");

    assert!(
        workflow.contains("pull_request"),
        "{SHELL_WORKFLOW} must run on pull_request — a check that only runs on a tag protects nothing (hub#933)",
    );
    assert!(
        workflow.contains(RUNNER_SCRIPT),
        "{SHELL_WORKFLOW} no longer invokes {RUNNER_SCRIPT}: the plugin's 34 Kotlin tests would go \
         back to running only in the release job, where a stale expectation waits for a tag (hub#933)",
    );
}

/// The workflow must keep watching the tree the Kotlin tests live in.
///
/// The path filter is what decides whether the job runs at all. Dropping
/// `crates/tauri-plugin-erplora-android/**` would leave the step present in the file and never
/// executed — a green PR that never compiled the thing it changed.
#[test]
fn the_workflow_still_watches_the_plugin_tree() {
    let workflow = code_of(&read(SHELL_WORKFLOW), "#");
    assert!(
        workflow.contains("crates/tauri-plugin-erplora-android/**"),
        "{SHELL_WORKFLOW} stopped watching the plugin tree: the Kotlin tests would be wired but \
         never triggered by the PRs that change them (hub#933)",
    );
}

/// The standalone project must NOT drag in `:app`.
///
/// `:app` is the half that cannot be configured without `cargo tauri android build` (it applies
/// `app/tauri.build.gradle.kts`, which that build writes). Including it is the difference between
/// a 22-second check and the 60-minute release job — i.e. between running on PRs and not.
#[test]
fn the_standalone_project_excludes_the_generated_app() {
    let settings = code_of(&read(STANDALONE_SETTINGS), "//");

    assert!(
        settings.contains(":tauri-plugin-erplora-android"),
        "{STANDALONE_SETTINGS} must include the plugin library — it is the project under test",
    );
    assert!(
        settings.contains(":tauri-android"),
        "{STANDALONE_SETTINGS} must include the tauri-android SDK: the plugin compiles against \
         `@TauriPlugin`, and one of its tests reads that annotation by reflection",
    );
    assert!(
        !settings.contains("\":app\"") && !settings.contains("':app'"),
        "{STANDALONE_SETTINGS} must NOT include `:app`: it applies `app/tauri.build.gradle.kts`, \
         which only `cargo tauri android build` writes, and re-chains the tests to the release",
    );
}

/// The standalone project must compile with the SAME toolchain as the release.
///
/// It repeats the versions of the generated root because that root cannot be imported. A drift
/// here is silent in the worst way: the PR check goes green having compiled the plugin with a
/// different AGP or Kotlin than the one that builds the shipped APK.
#[test]
fn the_standalone_project_pins_the_same_agp_and_kotlin_as_the_release() {
    let generated = read(GENERATED_ROOT_BUILD);
    let standalone = read(STANDALONE_BUILD);

    for artifact in [
        "com.android.tools.build:gradle",
        "org.jetbrains.kotlin:kotlin-gradle-plugin",
    ] {
        let expected = version_of(&generated, artifact).unwrap_or_else(|| {
            panic!("{GENERATED_ROOT_BUILD} no longer pins {artifact} — has the generator changed?")
        });
        let actual = version_of(&standalone, artifact).unwrap_or_else(|| {
            panic!("{STANDALONE_BUILD} must pin {artifact}, the release pins it at {expected}")
        });

        assert_eq!(
            expected, actual,
            "{artifact}: the release builds the plugin with {expected} and the PR check with \
             {actual}. The check would go green on a compiler the APK never sees — align \
             {STANDALONE_BUILD} with {GENERATED_ROOT_BUILD} (hub#933)",
        );
    }
}

/// Pulls `1.2.3` out of a `classpath("group:artifact:1.2.3")` line.
fn version_of(gradle: &str, artifact: &str) -> Option<String> {
    let needle = format!("{artifact}:");
    gradle.lines().find_map(|line| {
        let rest = line.split_once(&needle)?.1;
        let version: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        (!version.is_empty()).then_some(version)
    })
}
