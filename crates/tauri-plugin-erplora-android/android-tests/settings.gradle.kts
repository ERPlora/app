// Raíz Gradle mínima para correr los tests Kotlin del plugin SIN construir la app (hub#933).
//
// El proyecto de verdad vive en `apps/tauri/src-tauri/gen/android`, y no sirve para esto: su
// `settings.gradle` incluye `:app`, que aplica `app/tauri.build.gradle.kts` — un fichero que
// escribe `cargo tauri android build`. Por eso los 34 tests Kotlin del plugin solo se ejecutaban
// en la release: para llegar a un paso de 12 s había que pagar el build de Android entero (NDK,
// cuatro targets de Rust, ~60 min). Resultado: `48f2b0e1` dejó una expectativa caducada y
// `develop` estuvo un día en rojo sin que ningún check se enterara.
//
// Aquí solo entran los DOS proyectos que hacen falta para compilar y probar la librería:
// el plugin y el SDK `tauri-android` contra el que compila. Nada generado, nada de NDK, nada de
// Rust: 22 s en frío, 1 s en caliente.
//
// Se lanza con `scripts/kotlin-plugin-tests.sh`, que resuelve la ruta del SDK a partir del
// `tauri` que fija `Cargo.lock` — así esto se prueba SIEMPRE contra la misma versión que se
// publica, sin clavar aquí un número que se quedaría atrás en silencio.

rootProject.name = "erplora-android-plugin-tests"

val tauriAndroidDir: String =
    settings.startParameter.projectProperties["tauriAndroidDir"]
        ?: error(
            "falta -PtauriAndroidDir=<ruta al mobile/android del crate tauri>. " +
                "Usa scripts/kotlin-plugin-tests.sh, que la resuelve desde Cargo.lock.",
        )

include(":tauri-android")
project(":tauri-android").projectDir = file(tauriAndroidDir)

include(":tauri-plugin-erplora-android")
project(":tauri-plugin-erplora-android").projectDir = file("../android")
