// The plugin's Android module, included by the app's generated Gradle build and by the standalone
// test root in `android-tests/`. The policy classes are pure on purpose, so their tests run on the
// JVM with `testDebugUnitTest`: no emulator, seconds.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.erplora.android"
    compileSdk = 36

    defaultConfig {
        // The app runs on Android 7+; the permissions this plugin manages arrived in API 33 and 37,
        // and `PermissionPolicy` works out which ones apply at each level.
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    testOptions {
        unitTests {
            // `Build.VERSION.SDK_INT` is 0 on the JVM and the tests pass the level explicitly, so no
            // Robolectric is needed, only stub classes that return defaults instead of throwing.
            isReturnDefaultValues = true
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    // `AppCompatActivity` in `Plugin.onDestroy(activity)`, where the listening service stops and
    // the system voice shuts down; `PageKeeper` hears the activity's lifecycle through it too. The
    // same version the Tauri SDK already puts in the app, so the APK carries nothing new.
    implementation("androidx.appcompat:appcompat:1.6.0")
    // `WebViewCompat.getWebChromeClient` (WebPermissionGate): the platform getter is API 26, the
    // app starts at 24. Same version the app module already ships, so nothing new reaches the APK.
    implementation("androidx.webkit:webkit:1.14.0")
    implementation(project(":tauri-android"))

    testImplementation(kotlin("test"))
}
