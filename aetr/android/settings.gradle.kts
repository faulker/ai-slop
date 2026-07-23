// Gradle settings for the Aetr Android app. The app module wraps the shared
// aetr-core Rust crate (built via cargo-ndk, see app/build.gradle.kts).
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

rootProject.name = "Aetr"
include(":app")
