#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# The built APK declares no foreground service (ERPlora/app#23).
#
# Usage:  AAPT=<path to aapt2> scripts/ci/apk-foreground-service-guard.sh <apk>
#
# Google Play refuses a build that carries any `FOREGROUND_SERVICE*` permission
# until Play Console has its declaration, with a video of the feature running.
# The 1.1.33 got stuck there with `NoticeListeningService`, a service the Hub
# never started, so it was taken out. It comes back with the Hub's half
# (ERPlora/hub#836), the video and the declaration; until then a foreground
# service reaching the APK — ours, or one a library brings in through the
# manifest merger — is caught here, before any channel publishes.
#
# The MERGED manifest only exists inside the APK, so it is read back with aapt2:
#
#   · a FOREGROUND_SERVICE* permission (`uses-permission` or `-sdk-23`)  → RED
#   · a <service> with android:foregroundServiceType                     → RED
#   · the specialUse subtype property PROPERTY_SPECIAL_USE_FGS_SUBTYPE    → RED
#   · aapt2 fails, or answers without the package / <manifest>           → RED
#     (fail closed: an APK that cannot be read proves nothing)
#
# Exit: 0 none declared · 1 declared, or the APK could not be read · 2 usage.
# Tests: scripts/tests/apk-foreground-service-guard.test.sh
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

APK="${1:-}"
AAPT="${AAPT:-}"

if [ -z "$APK" ] || [ ! -f "$APK" ]; then
    echo "::error::usage: AAPT=<aapt2> $0 <apk> — no APK at '${APK}'"
    exit 2
fi
if [ -z "$AAPT" ] || [ ! -x "$AAPT" ]; then
    echo "::error::AAPT must point at an aapt2 binary (got '${AAPT}')"
    exit 2
fi

if ! permissions=$("$AAPT" dump permissions "$APK"); then
    echo "::error::aapt2 could not read the permissions of $APK: nothing proves it declares no foreground service"
    exit 1
fi
if ! manifest=$("$AAPT" dump xmltree --file AndroidManifest.xml "$APK"); then
    echo "::error::aapt2 could not read the manifest of $APK: nothing proves it declares no foreground service"
    exit 1
fi
if ! grep -q '^package: ' <<<"$permissions"; then
    echo "::error::aapt2 answered the permissions of $APK without its package line: not read as «nothing declared»"
    exit 1
fi
if ! grep -q 'E: manifest' <<<"$manifest"; then
    echo "::error::aapt2 answered the manifest of $APK without a <manifest> element: not read as «nothing declared»"
    exit 1
fi

found=0
why="Google Play refuses the build until it is declared with a video, and nothing turns it on until the Hub ports its half (ERPlora/hub#836, ERPlora/app#23)"

while IFS= read -r permission; do
    [ -n "$permission" ] || continue
    echo "::error::the APK declares $permission — $why"
    found=1
done < <(grep -oE "name='android\.permission\.FOREGROUND_SERVICE[A-Z_]*'" <<<"$permissions" | sed "s/^name='//; s/'\$//" | sort -u)

# The name of every <service> that carries a type. aapt2 prints an element's attributes on the
# lines after its `E:` line; the next element (a child or a sibling) ends the attribute list.
while IFS= read -r service; do
    [ -n "$service" ] || continue
    echo "::error::the APK declares the foreground service $service (android:foregroundServiceType) — $why"
    found=1
done < <(awk '
    /^ *E: / { in_service = ($2 == "service"); if (in_service) name = "(unnamed)"; next }
    in_service && /android:name\(/ { if (match($0, /="[^"]*"/)) name = substr($0, RSTART + 2, RLENGTH - 3) }
    in_service && /android:foregroundServiceType\(/ { print name }
' <<<"$manifest")

if grep -q 'android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE' <<<"$manifest"; then
    echo "::error::the APK declares android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE, the subtype of a specialUse foreground service — $why"
    found=1
fi

if [ "$found" -eq 0 ]; then
    echo "no foreground service in $APK (ERPlora/app#23)"
fi
exit "$found"
