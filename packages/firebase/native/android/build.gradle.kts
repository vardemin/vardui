// Firebase adapter for Vard: implements the `firebase` package's externs (firebase.vard)
// and feeds FCM into the std push events. The Android shell includes this library when the
// app depends on the package ([native.android] in vard.toml); VardActivity registers the
// module through its manifest meta-data.
plugins {
    id("com.android.library")
}

android {
    namespace = "dev.vard.firebase"
    compileSdk = 35
    defaultConfig {
        minSdk = 26
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":vard"))
    implementation(platform("com.google.firebase:firebase-bom:33.7.0"))
    implementation("com.google.firebase:firebase-auth")
    implementation("com.google.firebase:firebase-firestore")
    implementation("com.google.firebase:firebase-analytics")
    implementation("com.google.firebase:firebase-messaging")
}
