// Firebase adapter for Vard: implements knowledge/modules/firebase.vard (`vard add firebase`)
// and feeds FCM into the std push events. Registered automatically through manifest
// meta-data when the app depends on this library (-Pvard.firebase=true).
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
