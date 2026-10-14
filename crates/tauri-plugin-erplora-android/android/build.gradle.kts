// Módulo Android del plugin. Lo incluye el `settings.gradle` que genera
// `cargo tauri android init` en `apps/tauri/src-tauri/gen/android`.
//
// `PermissionPolicy` es lógica PURA a propósito (solo lee `Build.VERSION.SDK_INT` por defecto),
// así que sus tests corren en la JVM con `testDebugUnitTest` — sin emulador y en segundos.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.erplora.android"
    compileSdk = 36

    defaultConfig {
        // El shell apunta a Android 7+; los permisos que este plugin gestiona son de API 33 y 37,
        // y `PermissionPolicy` ya calcula cuáles aplican en cada nivel.
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
            // `Build.VERSION.SDK_INT` vale 0 en la JVM; los tests pasan el nivel explícitamente,
            // así que no hace falta Robolectric — pero sí que las clases stub no revienten.
            isReturnDefaultValues = true
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    // `AppCompatActivity` in `Plugin.onDestroy(activity)` (hub#2307). The same version the Tauri SDK
    // already puts in the app, so the APK carries nothing new.
    implementation("androidx.appcompat:appcompat:1.6.0")
    // El SDK del plugin de Tauri: `Plugin`, `@TauriPlugin`, `@Command`, `Invoke`.
    implementation(project(":tauri-android"))

    testImplementation(kotlin("test"))
}
