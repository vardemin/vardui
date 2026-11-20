// The Vard Android runtime as a library: libvard_ffi.so (Rust core) + the Kotlin host
// (VardView, VardHost, core modules, VardActivity). Apps and module libraries depend on it.
plugins {
    id("com.android.library")
}

val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile
val vardAbis: List<String> = providers.gradleProperty("vard.abis").getOrElse("arm64-v8a,x86_64")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }
val skipCargo: Boolean = providers.gradleProperty("vard.skipCargo").getOrElse("false").toBoolean()
val jniOut = layout.buildDirectory.dir("generated/vardJniLibs")

android {
    namespace = "dev.vard.host"
    compileSdk = 35
    ndkVersion = "28.2.13676358"

    defaultConfig {
        minSdk = 26
        ndk { abiFilters += vardAbis }
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
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
    // the GPU renderer (wgpu: Vulkan, GLES fallback); `vard.gpu=false` builds a Canvas-only library
    if ((project.findProperty("vard.gpu") as String?)?.toBoolean() != false) args += listOf("--features", "gpu")
    commandLine(args)
    val sdk = androidComponents.sdkComponents
    doFirst {
        environment("ANDROID_HOME", sdk.sdkDirectory.get().asFile.absolutePath)
        environment("ANDROID_NDK_HOME", sdk.ndkDirectory.get().asFile.absolutePath)
    }
}

tasks.named("preBuild") {
    if (!skipCargo) dependsOn(cargoNdk)
}
