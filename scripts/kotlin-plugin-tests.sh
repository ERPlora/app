#!/usr/bin/env bash
# Tests unitarios Kotlin del plugin de Android, SIN construir la app (hub#933).
#
# Los 34 tests de `crates/tauri-plugin-erplora-android/android/src/test` son donde vive la lógica
# de qué permiso se pide en cada nivel de API y cómo se habla con una impresora SPP. Hasta hub#933
# solo se ejecutaban en el job `build-android` de la release, porque el proyecto Gradle real
# (`gen/android`) no se puede ni configurar sin ficheros que escribe `cargo tauri android build`.
# Un test que solo corre al publicar no protege nada: el SPP (48f2b0e1) dejó una expectativa
# caducada y `develop` estuvo un día en rojo sin un solo check en ámbar.
#
# Este script usa la raíz mínima de `android-tests/`, que no incluye `:app` y por tanto no depende
# de nada generado. Necesita un JDK 17 y el SDK de Android (`platforms;android-36`); NO necesita
# NDK, ni Rust, ni tauri-cli.
#
# Uso:
#   scripts/kotlin-plugin-tests.sh                 # todos los tests del plugin
#   scripts/kotlin-plugin-tests.sh --tests '*Permission*'
#
# Sin awk/jq/python exóticos a propósito: `ci-runner-1` es un Ubuntu pelado sin `python` ni `gh`.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# La versión de `tauri` la manda Cargo.lock, no un número escrito aquí: el SDK Kotlin
# (`tauri-android`) viaja DENTRO del crate, así que probar contra otra versión sería probar contra
# un `@TauriPlugin` que no es el que se publica.
TAURI_VERSION=$(
    awk '
        /^name = "tauri"$/ { found = 1; next }
        found && /^version = / { gsub(/["]/, "", $3); print $3; exit }
    ' "$REPO_ROOT/Cargo.lock"
)

if [ -z "$TAURI_VERSION" ]; then
    echo "::error::no encuentro la versión de 'tauri' en Cargo.lock" >&2
    exit 1
fi

# El SDK sale del crate ya DESEMPAQUETADO en el registry. Lo desempaqueta el primer build que use
# `tauri`, que en CI es el `cargo test` que corre antes que este script.
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
TAURI_ANDROID_DIR=""
for candidate in "$CARGO_HOME"/registry/src/*/"tauri-$TAURI_VERSION"/mobile/android; do
    if [ -d "$candidate" ]; then
        TAURI_ANDROID_DIR="$candidate"
        break
    fi
done

if [ -z "$TAURI_ANDROID_DIR" ]; then
    echo "::error::no está el SDK Kotlin de tauri $TAURI_VERSION en $CARGO_HOME/registry/src." >&2
    echo "Se desempaqueta al compilar: corre antes 'cargo test -p erplora-tauri' (o 'cargo build -p erplora-tauri')." >&2
    exit 1
fi

echo "tauri-android SDK: $TAURI_ANDROID_DIR"

# Se reutiliza el wrapper del proyecto generado (Gradle 8.14.3) para probar con la MISMA versión
# de Gradle que la release, y para no commitear un segundo `gradle-wrapper.jar`. `-p` decide el
# proyecto; el wrapper solo aporta el binario.
exec "$REPO_ROOT/src-tauri/gen/android/gradlew" \
    -p "$REPO_ROOT/crates/tauri-plugin-erplora-android/android-tests" \
    -PtauriAndroidDir="$TAURI_ANDROID_DIR" \
    :tauri-plugin-erplora-android:testDebugUnitTest \
    "$@"
