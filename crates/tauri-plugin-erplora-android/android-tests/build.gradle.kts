import io.gitlab.arturbosch.detekt.DetektCreateBaselineTask
import io.gitlab.arturbosch.detekt.extensions.DetektExtension

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
        // Not one of the two: only this root runs detekt, the generated one never does.
        classpath("io.gitlab.arturbosch.detekt:detekt-gradle-plugin:1.23.8")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

// The function and class limits of the plugin's Kotlin code, run by `scripts/kotlin-plugin-tests.sh`.
// detekt is applied here and not in the plugin's own build file, which the app's generated Gradle
// build also reads and which has no detekt on its classpath.
apply(plugin = "io.gitlab.arturbosch.detekt")

val pluginSources = files("../android/src/main/java", "../android/src/test/java")
val limitsConfig = files("detekt.yml")
val excusedCode = file("detekt-baseline.xml")

configure<DetektExtension> {
    source.setFrom(pluginSources)
    config.setFrom(limitsConfig)
    buildUponDefaultConfig = true
    baseline = excusedCode
}

// detekt skips a baseline line that no longer matches any code, so a fixed function would stay
// excused forever. This rebuilds the baseline from today's code and fails while the committed one
// still excuses something that keeps the limits now.
val currentFindings = tasks.register<DetektCreateBaselineTask>("detektCurrentFindings") {
    setSource(pluginSources)
    config.setFrom(limitsConfig)
    buildUponDefaultConfig.set(true)
    baseline.set(layout.buildDirectory.file("detekt/current-baseline.xml"))
}

tasks.register("detektBaselineUpToDate") {
    val current = currentFindings.flatMap { it.baseline }
    dependsOn(currentFindings)
    doLast {
        // detekt writes no file when it finds nothing.
        val idsOf = { xml: File ->
            if (xml.exists()) {
                Regex("<ID>(.+?)</ID>").findAll(xml.readText()).map { it.groupValues[1] }.toSet()
            } else {
                emptySet()
            }
        }
        val fixed = idsOf(excusedCode) - idsOf(current.get().asFile)
        if (fixed.isNotEmpty()) {
            throw GradleException(
                "detekt-baseline.xml still excuses code that keeps the limits now; remove: " +
                    fixed.joinToString("; "),
            )
        }
    }
}
