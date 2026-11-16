pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "vard-android"
// :vard — runtime + host library; :app — the shell that bundles a Vard project.
include(":vard", ":app")
// Optional module libraries (each adapts a native SDK to Vard `extern fn`s / events).
if (providers.gradleProperty("vard.firebase").getOrElse("false").toBoolean()) include(":vard-firebase")
