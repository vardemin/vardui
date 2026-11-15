plugins {
    id("com.android.application")
}

// Repository root (platforms/android/../..), the Vard project to bundle and the ABIs to build.
val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile
val vardApp: String = providers.gradleProperty("vard.app").getOrElse("examples/todo")
val vardAbis: List<String> = providers.gradleProperty("vard.abis").getOrElse("arm64-v8a,x86_64")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }
val skipCargo: Boolean = providers.gradleProperty("vard.skipCargo").getOrElse("false").toBoolean()

val jniOut = layout.buildDirectory.dir("generated/vardJniLibs")
val assetsOut = layout.buildDirectory.dir("generated/vardAssets")

android {
    namespace = "dev.vard.host"
    compileSdk = 35
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "dev.vard.host"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
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
            if (!skipCargo) jniLibs.directories += jniOut.get().asFile.path
        }
    }
}

// Build libvard_ffi.so for every ABI with cargo-ndk (cargo itself is incremental).
val cargoNdk by tasks.registering(Exec::class) {
    description = "Builds crates/vard-ffi for Android with cargo-ndk"
    workingDir = repoRoot
    val out = jniOut.get().asFile
    val args = mutableListOf("cargo", "ndk")
    vardAbis.forEach { args += listOf("-t", it) }
    args += listOf("-P", "26", "-o", out.absolutePath, "build", "--release", "-p", "vard-ffi")
    commandLine(args)
    val sdk = androidComponents.sdkComponents
    doFirst {
        environment("ANDROID_HOME", sdk.sdkDirectory.get().asFile.absolutePath)
        environment("ANDROID_NDK_HOME", sdk.ndkDirectory.get().asFile.absolutePath)
    }
}

// Bundle the Vard sources as assets/app/*.vard (tests and other files are left out).
val copyVardSources by tasks.registering(Sync::class) {
    description = "Copies the Vard project ($vardApp) into the APK assets"
    from(repoRoot.resolve(vardApp)) {
        include("*.vard")
    }
    into(assetsOut.map { it.dir("app") })
}

tasks.named("preBuild") {
    dependsOn(copyVardSources)
    if (!skipCargo) dependsOn(cargoNdk)
}
