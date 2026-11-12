// Minimal Gradle root to run the plugin's Kotlin tests WITHOUT building the app.
//
// The real project lives in `src-tauri/gen/android` and does not serve for this: its
// `settings.gradle` includes `:app`, which applies `app/tauri.build.gradle.kts` — a file that
// `cargo tauri android build` writes. Through it, reaching a 12 s test step means paying for the
// whole Android build (NDK, four Rust targets, ~60 min).
//
// Only the TWO projects needed to compile and test the library come in here: the plugin and the
// `tauri-android` SDK it compiles against. Nothing generated, no NDK, no Rust: 22 s cold, 1 s warm.
//
// It is launched by `scripts/kotlin-plugin-tests.sh`, which resolves the SDK path from the `tauri`
// that `Cargo.lock` pins — so this is ALWAYS tested against the same version that ships, without
// pinning a number here that would silently fall behind.

rootProject.name = "erplora-android-plugin-tests"

val tauriAndroidDir: String =
    settings.startParameter.projectProperties["tauriAndroidDir"]
        ?: error(
            "missing -PtauriAndroidDir=<path to the tauri crate's mobile/android>. " +
                "Use scripts/kotlin-plugin-tests.sh, which resolves it from Cargo.lock.",
        )

include(":tauri-android")
project(":tauri-android").projectDir = file(tauriAndroidDir)

include(":tauri-plugin-erplora-android")
project(":tauri-plugin-erplora-android").projectDir = file("../android")
