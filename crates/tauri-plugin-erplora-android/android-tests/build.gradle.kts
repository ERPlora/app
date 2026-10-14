// Toolchain de la raíz de tests del plugin (hub#933).
//
// 🔴 Estas DOS versiones son las mismas que las de
// `apps/tauri/src-tauri/gen/android/build.gradle.kts`, y tienen que seguir siéndolo: si divergen,
// el check del PR compila el Kotlin del plugin con un compilador DISTINTO del que construye el
// APK que se publica — verde aquí, roto allí, y sin que nada lo diga. No se pueden importar de
// allí (ese fichero lo genera `cargo tauri android init` y su `buildscript` no se puede aplicar
// desde fuera), así que se repiten a mano y lo que impide la deriva es un test:
// `apps/tauri/src-tauri/tests/android_kotlin_tests_run_on_pr.rs`. Si tocas una versión allí,
// tócala aquí — el test te lo va a decir de todas formas.
buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:8.11.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.25")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}
