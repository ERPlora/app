#!/usr/bin/env bash
# Kotlin unit tests of the Android plugin, WITHOUT building the app.
#
# The tests under `crates/tauri-plugin-erplora-android/android/src/test` hold which permission is
# asked for at each API level and how an SPP printer is spoken to. The real Gradle project
# (`gen/android`) cannot even be configured without files that `cargo tauri android build` writes,
# so this script uses the minimal root `android-tests/`, which has no `:app` and therefore depends
# on nothing generated. It needs a JDK 17 and the Android SDK (`platforms;android-36`); it does NOT
# need the NDK, Rust or tauri-cli.
#
# Usage:
#   scripts/kotlin-plugin-tests.sh                 # every test of the plugin
#   scripts/kotlin-plugin-tests.sh --tests '*Permission*'
#
# No awk/jq/python beyond the basics on purpose: a bare Ubuntu runner has neither `python` nor `gh`.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# Cargo.lock decides the `tauri` version, not a number written here: the Kotlin SDK
# (`tauri-android`) travels INSIDE the crate, so testing against another version would test against
# a `@TauriPlugin` that is not the one shipped.
TAURI_VERSION=$(
    awk '
        /^name = "tauri"$/ { found = 1; next }
        found && /^version = / { gsub(/["]/, "", $3); print $3; exit }
    ' "$REPO_ROOT/Cargo.lock"
)

if [ -z "$TAURI_VERSION" ]; then
    echo "::error::cannot find the version of 'tauri' in Cargo.lock" >&2
    exit 1
fi

# The SDK comes from the crate already UNPACKED in the registry. The first build that uses `tauri`
# unpacks it; in CI that is the `cargo test` that runs before this script.
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
TAURI_ANDROID_DIR=""
for candidate in "$CARGO_HOME"/registry/src/*/"tauri-$TAURI_VERSION"/mobile/android; do
    if [ -d "$candidate" ]; then
        TAURI_ANDROID_DIR="$candidate"
        break
    fi
done

if [ -z "$TAURI_ANDROID_DIR" ]; then
    echo "::error::the Kotlin SDK of tauri $TAURI_VERSION is not in $CARGO_HOME/registry/src." >&2
    echo "It is unpacked by a build: run 'cargo test -p erplora-tauri' (or 'cargo build -p erplora-tauri') first." >&2
    exit 1
fi

echo "tauri-android SDK: $TAURI_ANDROID_DIR"

# The generated project's wrapper (Gradle 8.14.3) is reused, to test with the SAME Gradle version
# as the release and to avoid committing a second `gradle-wrapper.jar`. `-p` picks the project; the
# wrapper only brings the binary.
exec "$REPO_ROOT/src-tauri/gen/android/gradlew" \
    -p "$REPO_ROOT/crates/tauri-plugin-erplora-android/android-tests" \
    -PtauriAndroidDir="$TAURI_ANDROID_DIR" \
    :tauri-plugin-erplora-android:testDebugUnitTest \
    "$@"
