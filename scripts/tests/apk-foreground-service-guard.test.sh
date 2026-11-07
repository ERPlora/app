#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for scripts/ci/apk-foreground-service-guard.sh — the built APK declares
# no foreground service (ERPlora/app#23).
#
# Run:  bash scripts/tests/apk-foreground-service-guard.test.sh
#
# Google Play refuses a build with any `FOREGROUND_SERVICE*` permission until it
# is declared in Play Console with a video of the feature running. The 1.1.33
# got stuck there with a service the Hub never starts. The merged manifest only
# exists inside the APK, so the release job reads it back with aapt2 after the
# build and before anything is published:
#
#   · a FOREGROUND_SERVICE* permission (any spelling aapt2 prints)  → RED
#   · a <service> with android:foregroundServiceType                 → RED
#   · the specialUse subtype property                                → RED
#   · none of them (a plain service is fine)                         → go
#   · aapt2 fails, or answers without the package / manifest         → RED
#     (fail closed: an unreadable APK proves nothing)
#
# The fixtures are aapt2 2.20's real output for the 1.1.33 APK of the release
# `v1.1.33` (`dump permissions`, `dump xmltree --file AndroidManifest.xml`).
# `aapt2` is a fake: no Android SDK needed.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/apk-foreground-service-guard.sh"
FIXTURES="$script_dir/fixtures/apk-1.1.33"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-apk-fgs-guard-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

# The fake `aapt2`: FAKE_PERMISSIONS and FAKE_XMLTREE are the files it prints; FAKE_FAIL makes one
# of the two dumps fail AFTER printing it, so only its exit code says the answer is not to be
# trusted. Every call is logged.
cat > "$tmp_dir/aapt2" <<'FAKE'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
case "$1 $2" in
    "dump permissions") cat "$FAKE_PERMISSIONS"; if [ "${FAKE_FAIL:-}" = permissions ]; then echo "error: truncated zip" >&2; exit 1; fi ;;
    "dump xmltree")     cat "$FAKE_XMLTREE"; if [ "${FAKE_FAIL:-}" = xmltree ]; then echo "error: truncated zip" >&2; exit 1; fi ;;
    *) echo "fake aapt2: unexpected call: $*" >&2; exit 64 ;;
esac
FAKE
chmod +x "$tmp_dir/aapt2"
: > "$tmp_dir/app.apk"

# The 1.1.33 APK with the listening service taken out the way app#23 takes it out: no permissions,
# no type, no subtype. The <service> tag itself stays on purpose — a plain service is not a
# foreground one (Android 14+ refuses `startForeground` without a declared type).
grep -v -e "FOREGROUND_SERVICE" "$FIXTURES/permissions.txt" > "$tmp_dir/permissions.clean"
grep -v -e "foregroundServiceType" -e "PROPERTY_SPECIAL_USE_FGS_SUBTYPE" -e "Point of sale that must hear" \
    "$FIXTURES/xmltree.txt" > "$tmp_dir/xmltree.clean"

run() {
    : > "$tmp_dir/log"
    out=$(
        FAKE_LOG="$tmp_dir/log" FAKE_PERMISSIONS="$1" FAKE_XMLTREE="$2" FAKE_FAIL="${FAIL-}" \
        AAPT="${AAPT_BIN-$tmp_dir/aapt2}" bash "$GUARD" "${APK_ARG-$tmp_dir/app.apk}" 2>&1
    )
    status=$?
    calls=$(cat "$tmp_dir/log")
}
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }

echo
echo "apk-foreground-service-guard.sh (the built APK declares no foreground service, app#23)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/apk-foreground-service-guard.sh exists"
else
    bad "scripts/ci/apk-foreground-service-guard.sh exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

# ── The 1.1.33 as it shipped: the positive the guard exists for ─────────────────────────────────
run "$FIXTURES/permissions.txt" "$FIXTURES/xmltree.txt"
expect_status "the 1.1.33 APK (FOREGROUND_SERVICE + _SPECIAL_USE + a specialUse service) → RED" 1
expect_says "…naming FOREGROUND_SERVICE" "declares android.permission.FOREGROUND_SERVICE —"
expect_says "…naming FOREGROUND_SERVICE_SPECIAL_USE" "declares android.permission.FOREGROUND_SERVICE_SPECIAL_USE —"
expect_says "…naming the service that carries a type" "com.erplora.android.NoticeListeningService"
expect_says "…as a GitHub error annotation" "::error::"
case "$calls" in
    *"dump permissions $tmp_dir/app.apk"*"dump xmltree --file AndroidManifest.xml $tmp_dir/app.apk"*)
        ok "…it read the permissions and the manifest of THAT apk" ;;
    *) bad "…it read the permissions and the manifest of THAT apk" "calls were: $calls" ;;
esac

# ── The same APK with the service taken out (what app#23 ships) ─────────────────────────────────
run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "no foreground permission, no type, no subtype → go" 0

# ── Each half on its own is enough to be refused ────────────────────────────────────────────────
{ cat "$tmp_dir/permissions.clean"; echo "uses-permission: name='android.permission.FOREGROUND_SERVICE'"; } > "$tmp_dir/p.general"
run "$tmp_dir/p.general" "$tmp_dir/xmltree.clean"
expect_status "only the general FOREGROUND_SERVICE permission → RED" 1

{ cat "$tmp_dir/permissions.clean"; echo "uses-permission-sdk-23: name='android.permission.FOREGROUND_SERVICE_DATA_SYNC'"; } > "$tmp_dir/p.sdk23"
run "$tmp_dir/p.sdk23" "$tmp_dir/xmltree.clean"
expect_status "a typed variant declared as uses-permission-sdk-23 → RED" 1
expect_says "…naming it" "android.permission.FOREGROUND_SERVICE_DATA_SYNC"

grep -v -e "PROPERTY_SPECIAL_USE_FGS_SUBTYPE" -e "Point of sale that must hear" "$FIXTURES/xmltree.txt" > "$tmp_dir/x.typed"
run "$tmp_dir/permissions.clean" "$tmp_dir/x.typed"
expect_status "a <service> with foregroundServiceType and no permission → RED" 1
expect_says "…naming the service" "com.erplora.android.NoticeListeningService"

grep -v -e "foregroundServiceType" "$FIXTURES/xmltree.txt" > "$tmp_dir/x.subtype"
run "$tmp_dir/permissions.clean" "$tmp_dir/x.subtype"
expect_status "the specialUse subtype property alone → RED" 1
expect_says "…naming it" "PROPERTY_SPECIAL_USE_FGS_SUBTYPE"

# ── Fail closed: an APK that cannot be read proves nothing ──────────────────────────────────────
# Clean answers on purpose: the failed exit code is the only thing that can make these red.
FAIL=permissions run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "aapt2 fails reading the permissions, even after printing clean ones → RED" 1
FAIL=xmltree run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "aapt2 fails reading the manifest, even after printing a clean one → RED" 1

: > "$tmp_dir/empty"
run "$tmp_dir/empty" "$tmp_dir/xmltree.clean"
expect_status "the permissions answer has no package line → RED, never «nothing declared»" 1
run "$tmp_dir/permissions.clean" "$tmp_dir/empty"
expect_status "the manifest answer has no <manifest> element → RED, never «nothing declared»" 1

# ── Usage ───────────────────────────────────────────────────────────────────────────────────────
APK_ARG= run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "no APK given → usage error (exit 2)" 2
APK_ARG="$tmp_dir/missing.apk" run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "an APK that does not exist → usage error (exit 2)" 2
AAPT_BIN= run "$tmp_dir/permissions.clean" "$tmp_dir/xmltree.clean"
expect_status "no aapt2 given → usage error (exit 2)" 2

echo
printf 'passed: %d   failed: %d\n\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
