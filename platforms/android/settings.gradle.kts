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

// Native modules of the app's packages ([native.android] gradle = "..." in each package's
// vard.toml): `vard android-modules` resolves the app's dependencies and prints
// `name<TAB>gradle dir<TAB>module class`; each becomes the library project `:vard-<name>`.
// The list is passed to :app through gradle.extra["vardModules"].
val repoRoot: File = settingsDir.resolve("../..").canonicalFile
val vardApp: String = providers.gradleProperty("vard.app").getOrElse("examples/todo")
val vardModules: List<String> = run {
    val out = providers.exec {
        workingDir = repoRoot
        commandLine("cargo", "run", "-q", "--release", "-p", "vard-cli", "--", "android-modules", vardApp)
        isIgnoreExitValue = true
    }.standardOutput.asText.get()
    out.lines().filter { it.contains('\t') }.map { line ->
        val (name, dir) = line.split('\t')
        include(":vard-$name")
        project(":vard-$name").projectDir = file(dir)
        ":vard-$name"
    }
}
gradle.extra["vardModules"] = vardModules
