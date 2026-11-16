plugins {
    id("com.android.application")
}

// Repository root (platforms/android/../..) and the Vard project to bundle.
val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile
val vardApp: String = providers.gradleProperty("vard.app").getOrElse("examples/todo")
val vardAbis: List<String> = providers.gradleProperty("vard.abis").getOrElse("arm64-v8a,x86_64")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }
val firebase: Boolean = providers.gradleProperty("vard.firebase").getOrElse("false").toBoolean()

val assetsOut = layout.buildDirectory.dir("generated/vardAssets")

if (firebase && file("google-services.json").exists()) {
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
        versionName = "0.2.0"
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
    if (firebase) implementation(project(":vard-firebase"))
}

// Bundle the Vard project as assets/app: sources, fonts/ and assets/ (tests are left out).
val copyVardSources by tasks.registering(Sync::class) {
    description = "Copies the Vard project ($vardApp) into the APK assets"
    from(repoRoot.resolve(vardApp)) {
        include("**/*.vard", "fonts/**", "assets/**")
        exclude("tests/**", "build/**")
    }
    into(assetsOut.map { it.dir("app") })
}

tasks.named("preBuild") {
    dependsOn(copyVardSources)
}
