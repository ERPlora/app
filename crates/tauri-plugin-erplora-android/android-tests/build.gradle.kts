// Toolchain of the plugin's test root.
//
// 🔴 These TWO versions are the same as those of `src-tauri/gen/android/build.gradle.kts`, and they
// must stay so: if they drift, the PR check compiles the plugin's Kotlin with a DIFFERENT compiler
// from the one that builds the published APK — green here, broken there, and nothing says so. They
// cannot be imported from there (`cargo tauri android init` generates that file and its
// `buildscript` cannot be applied from outside), so they are repeated by hand and a test keeps them
// from drifting: `src-tauri/tests/android_kotlin_tests_run_on_pr.rs`. If you change a version
// there, change it here — the test will tell you anyway.
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
