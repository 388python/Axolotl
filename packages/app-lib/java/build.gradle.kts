import java.security.MessageDigest

plugins {
    java
    id("com.diffplug.spotless") version "8.0.0"
    id("com.gradleup.shadow") version "9.2.2"
}

providers.gradleProperty("axolotlBuildDir").orNull?.let {
    layout.buildDirectory.set(file(it))
}

repositories {
    mavenCentral()
}

dependencies {
    implementation("org.ow2.asm:asm:9.9")
    implementation("org.ow2.asm:asm-tree:9.9")
    implementation("com.google.code.gson:gson:2.13.2")

    testImplementation(libs.junit.jupiter)
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(17)
    }
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 8
    options.compilerArgs.addAll(listOf("-Xlint:all", "-Werror"))
}

spotless {
    java {
        palantirJavaFormat()
        removeUnusedImports()
    }
}

tasks.jar {
    enabled = false
}

tasks.shadowJar {
    archiveFileName = "theseus.jar"
    manifest {
        attributes["Premain-Class"] = "com.modrinth.theseus.agent.TheseusAgent"
    }

    addMultiReleaseAttribute = false
    enableAutoRelocation = true
    relocationPrefix = "com.modrinth.theseus.shadow"
}

val authlibInjector by tasks.registering {
    notCompatibleWithConfigurationCache("Downloads and verifies a pinned external Java agent")
    val output = layout.buildDirectory.file("libs/authlib-injector.jar")
    val expectedSha256 =
        "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10"
    inputs.property("version", "1.2.8")
    inputs.property("sha256", expectedSha256)
    outputs.file(output)

    doLast {
        val target = output.get().asFile
        // The download is pinned by checksum, so an existing file that still
        // matches it is the same artifact and does not need fetching again.
        // Without this check every build redownloads the agent, which fails
        // outright on machines that cannot reach the artifact host.
        if (target.exists()) {
            val existing = MessageDigest.getInstance("SHA-256")
                .digest(target.readBytes())
                .joinToString("") { "%02x".format(it) }
            if (existing == expectedSha256) return@doLast
        }
        val bytes = uri(
            "https://authlib-injector.yushi.moe/artifact/56/authlib-injector-1.2.8.jar",
        ).toURL().readBytes()
        val checksum = MessageDigest.getInstance("SHA-256")
            .digest(bytes)
            .joinToString("") { "%02x".format(it) }
        check(checksum == expectedSha256) {
            "authlib-injector checksum mismatch: $checksum"
        }
        target.apply {
            parentFile.mkdirs()
            writeBytes(bytes)
        }
    }
}

tasks.build {
    dependsOn(authlibInjector)
}

tasks.named<Test>("test") {
    useJUnitPlatform()
}

tasks.withType<AbstractArchiveTask>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}
