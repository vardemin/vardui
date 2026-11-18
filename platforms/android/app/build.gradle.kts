plugins {
    id("com.android.application")
}

// Repository root (platforms/android/../..) and the Vard project to bundle.
val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile
val vardApp: String = providers.gradleProperty("vard.app").getOrElse("examples/todo")
val vardAbis: List<String> = providers.gradleProperty("vard.abis").getOrElse("arm64-v8a,x86_64")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }
// native modules of the app's packages, found by settings.gradle.kts
@Suppress("UNCHECKED_CAST")
val vardModules: List<String> = (gradle.extra["vardModules"] as? List<String>) ?: emptyList()

val assetsOut = layout.buildDirectory.dir("generated/vardAssets")

if (":vard-firebase" in vardModules && file("google-services.json").exists()) {
    apply(plugin = "com.google.gms.google-services")
}

android {
    namespace = "dev.vard.app"
    compileSdk = 35

    defaultConfig {
        applicationId = providers.gradleProperty("vard.applicationId").getOrElse("dev.vard.host")
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.4.0"
        ndk { abiFilters += vardAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            assets.directories += assetsOut.get().asFile.path
        }
    }
}

dependencies {
    implementation(project(":vard"))
    vardModules.forEach { implementation(project(it)) }
}

// Bundle the Vard project as assets/app with `vard bundle`: app sources, its packages
// (`@name/...`), vard.lock, fonts/, assets/ and bundle.json (hash + native fingerprint).
val bundleVardApp by tasks.registering(Exec::class) {
    description = "Bundles the Vard project ($vardApp) and its packages into the APK assets"
    workingDir = repoRoot
    val out = assetsOut.map { it.dir("app") }
    doFirst { out.get().asFile.deleteRecursively() }
    commandLine("cargo", "run", "-q", "--release", "-p", "vard-cli", "--", "bundle", vardApp, "--out", out.get().asFile.absolutePath)
}

tasks.named("preBuild") {
    dependsOn(bundleVardApp)
}
