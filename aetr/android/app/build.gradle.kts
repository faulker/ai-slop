import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Aetr app module. Builds the shared aetr-core Rust crate for Android ABIs
// via cargo-ndk before every build and packages the resulting cdylib
// (libaetr_core.so) in jniLibs; the UniFFI-generated Kotlin bindings in
// src/main/java/uniffi/aetr_core load it through JNA.
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "me.faulk.aetr"
    compileSdk = 35

    defaultConfig {
        applicationId = "me.faulk.aetr"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1"
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests.all {
            // Lets the JVM unit test load a host-built libaetr_core
            // (produced by scripts/gen-bindings.sh) through JNA.
            it.systemProperty(
                "jna.library.path",
                rootProject.projectDir.parentFile.resolve("target/release").absolutePath
            )
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

// Builds the aetr-core cdylib for both device (arm64-v8a) and emulator
// (x86_64) ABIs into src/main/jniLibs. Runs from the aetr workspace root so
// cargo resolves the workspace; the NDK is discovered via ANDROID_HOME.
// The core's C++ modem shim needs the C++ runtime. We link the *shared*
// libc++ (NDK's supported default) and bundle libc++_shared.so alongside
// libaetr_core.so in jniLibs. The earlier c++_static attempt left the C++ABI
// symbols (__gxx_personality_v0, __cxa_*_catch) unresolved because rustc's
// final link (via clang, not clang++) pulled in libc++_static.a but not
// libc++abi.a, so dlopen failed at runtime.
val abiTriples = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android"
)

val cargoNdk = tasks.register<Exec>("cargoNdk") {
    group = "build"
    description = "Cross-compiles aetr-core for Android ABIs into jniLibs"
    workingDir = rootProject.projectDir.parentFile
    val home = System.getProperty("user.home")
    val androidHome = System.getenv("ANDROID_HOME") ?: "$home/Library/Android/sdk"
    environment("PATH", "$home/.cargo/bin:${System.getenv("PATH")}")
    environment("ANDROID_HOME", androidHome)
    // Link the shared C++ stdlib; libc++_shared.so is copied into jniLibs below.
    environment("CXXSTDLIB", "c++_shared")
    commandLine(
        "$home/.cargo/bin/cargo", "ndk",
        "-t", "arm64-v8a", "-t", "x86_64",
        "-o", "android/app/src/main/jniLibs",
        "build", "--release", "-p", "aetr-core"
    )

    // Copy the NDK's libc++_shared.so into each ABI dir so the packaged
    // libaetr_core.so can resolve its C++ runtime dependency at load time.
    doLast {
        val ndkRoot = System.getenv("ANDROID_NDK_HOME")
            ?: file("$androidHome/ndk").listFiles()
                ?.filter { it.isDirectory }
                ?.maxByOrNull { it.name }?.absolutePath
            ?: error("No NDK found under $androidHome/ndk; set ANDROID_NDK_HOME")
        // Only darwin-x86_64 / linux-x86_64 host prebuilts exist; pick whichever is present.
        val hostLib = file("$ndkRoot/toolchains/llvm/prebuilt")
            .listFiles()?.firstOrNull { it.isDirectory }
            ?.resolve("sysroot/usr/lib")
            ?: error("NDK sysroot lib dir not found under $ndkRoot")
        abiTriples.forEach { (abi, triple) ->
            val src = hostLib.resolve("$triple/libc++_shared.so")
            val dstDir = file("src/main/jniLibs/$abi")
            require(src.exists()) { "Missing $src" }
            dstDir.mkdirs()
            src.copyTo(dstDir.resolve("libc++_shared.so"), overwrite = true)
        }
    }
}

tasks.named("preBuild") {
    dependsOn(cargoNdk)
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.05.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")

    // JNA backs the UniFFI bindings; @aar bundles the Android native dispatch libs.
    implementation("net.java.dev.jna:jna:5.17.0@aar")

    // Host-JVM JNA for the desktop loopback unit test.
    testImplementation("net.java.dev.jna:jna:5.17.0")
    testImplementation("junit:junit:4.13.2")
}
