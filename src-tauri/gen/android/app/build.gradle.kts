import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.tomlj.Toml

buildscript {
    repositories { mavenCentral() }
    dependencies { classpath("org.tomlj:tomlj:1.1.1") }
}

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

val workspaceVersion by lazy {
    val cargoFile = rootProject.file("../../../Cargo.toml")
    if (!cargoFile.isFile) throw GradleException("Workspace manifest is missing: $cargoFile")
    val manifest = Toml.parse(cargoFile.toPath())
    if (manifest.hasErrors()) {
        throw GradleException("Invalid workspace manifest $cargoFile: ${manifest.errors()}")
    }
    val version = manifest.get("workspace.package.version") as? String
        ?: throw GradleException("Expected [workspace.package].version to be a string in $cargoFile")
    val number = "(0|[1-9][0-9]*)"
    val identifiers = "[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*"
    val semver = Regex("$number\\.$number\\.$number(?:-($identifiers))?(?:\\+$identifiers)?")
    val parsed = semver.matchEntire(version)
        ?: throw GradleException("Invalid workspace SemVer in $cargoFile: $version")
    val prerelease = parsed.groupValues[4]
    if (prerelease.split('.').any { it.length > 1 && it.startsWith('0') && it.all(Char::isDigit) }) {
        throw GradleException("Invalid workspace SemVer in $cargoFile: $version")
    }
    parsed
}

fun workspaceVersionCode(version: MatchResult): Int {
    val (_, majorText, minorText, patchText, prerelease) = version.groupValues
    val major = majorText.toIntOrNull()
    val minor = minorText.toIntOrNull()
    val patch = patchText.toIntOrNull()
    if (major == null || major > 210 || minor == null || minor > 99 || patch == null || patch > 99) {
        throw GradleException("Workspace version ${version.value} exceeds the Android versionCode fields")
    }
    val numbered = Regex("(alpha|beta|rc)\\.([0-9]+)").matchEntire(prerelease)
    val iteration = numbered?.groupValues?.get(2)?.toIntOrNull() ?: Int.MAX_VALUE
    val preOffset = when (numbered?.groupValues?.get(1)) {
        "alpha" -> 100 + iteration.coerceAtMost(199)
        "beta" -> 300 + iteration.coerceAtMost(299)
        "rc" -> 600 + iteration.coerceAtMost(299)
        else -> if (prerelease.isEmpty()) 999 else 1
    }
    val code = major * 10_000_000 + minor * 100_000 + patch * 1_000 + preOffset
    if (code > 2_100_000_000) {
        throw GradleException("Workspace version ${version.value} exceeds the Android versionCode limit")
    }
    return code
}

val keystoreProperties = Properties().apply {
    val propFile = rootProject.file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

fun signingProp(propKey: String, envKey: String): String? =
    System.getenv(envKey)?.takeIf { it.isNotBlank() }
        ?: keystoreProperties.getProperty(propKey)?.takeIf { it.isNotBlank() }

val releaseKeystorePath = signingProp("storeFile", "ROVETERM_ANDROID_KEYSTORE_PATH")
val releaseKeystorePassword = signingProp("storePassword", "ROVETERM_ANDROID_KEYSTORE_PASSWORD")
val releaseKeyAlias = signingProp("keyAlias", "ROVETERM_ANDROID_KEY_ALIAS")
val releaseKeyPassword = signingProp("keyPassword", "ROVETERM_ANDROID_KEY_PASSWORD")
val hasReleaseSigning =
    releaseKeystorePath != null &&
        releaseKeystorePassword != null &&
        releaseKeyAlias != null &&
        releaseKeyPassword != null

android {
    compileSdk = 37
    namespace = "dev.roveterm.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "dev.roveterm.app"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode")?.toInt()
            ?: workspaceVersionCode(workspaceVersion)
        versionName = tauriProperties.getProperty("tauri.android.versionName") ?: workspaceVersion.value
    }
    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(releaseKeystorePath!!)
                storePassword = releaseKeystorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
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

apply(from = file("tauri.build.gradle.kts"))
