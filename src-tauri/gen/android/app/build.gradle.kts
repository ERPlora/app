import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Firma de release. El fichero lo escribe el CI desde los secretos del repo
// (ANDROID_KEYSTORE_BASE64 / ANDROID_KEYSTORE_PASSWORD, alias `erplora`, ADR-0053) y NO está en
// git. Si no existe —build local— la release sale sin firmar en vez de romper el build.
val keystoreProperties = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

/**
 * `versionCode` según ADR-0160: `major*1_000_000 + minor*1_000 + patch`.
 *
 * NO se usa el que calcula Tauri por su cuenta: Play es una **puerta de un solo sentido** —un
 * `versionCode` publicado no se puede bajar ni reutilizar jamás. Si se sube uno con la fórmula
 * equivocada, ese número queda quemado para siempre y la numeración del ADR ya no se puede
 * aplicar sin saltos.
 */
fun versionCodeFrom(name: String): Int {
    val p = Regex("""^(\d+)\.(\d+)\.(\d+)""").find(name)?.destructured
        ?: return tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
    val (major, minor, patch) = p
    return major.toInt() * 1_000_000 + minor.toInt() * 1_000 + patch.toInt()
}

android {
    compileSdk = 36
    namespace = "com.erplora.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.erplora.app"
        minSdk = 24
        targetSdk = 36
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
        versionCode = versionCodeFrom(versionName!!)
    }
    signingConfigs {
        create("release") {
            keystoreProperties.getProperty("storeFile")?.let {
                storeFile = file(it)
                storePassword = keystoreProperties.getProperty("password")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("password")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // Sin `keystore.properties` (build local) se queda sin firmar en vez de romper.
            if (keystoreProperties.getProperty("storeFile") != null) {
                signingConfig = signingConfigs.getByName("release")
            }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")