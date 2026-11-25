fn main() {
    println!("cargo:rerun-if-changed=native.m");
    println!("cargo:rerun-if-changed=activity.m");
    println!("cargo:rerun-if-changed=meeting.m");
    println!("cargo:rerun-if-changed=voice.m");
    println!("cargo:rerun-if-changed=overlay.m");
    cc::Build::new()
        .file("native.m")
        .file("activity.m")
        .file("meeting.m")
        .file("voice.m")
        .file("overlay.m")
        .flag("-fobjc-arc")
        .flag("-Wno-incompatible-pointer-types")
        .compile("sumi_native");
    for name in [
        "Foundation",
        "Vision",
        "AVFoundation",
        "CoreMedia",
        "Speech",
        "Carbon",
        "ScreenCaptureKit",
        "ServiceManagement",
        "ImageIO",
        "AppKit",
        "QuartzCore",
        "PDFKit",
        "NaturalLanguage",
        "ApplicationServices",
        "IOKit",
        "UserNotifications",
    ] {
        println!("cargo:rustc-link-lib=framework={name}");
    }
    tauri_build::build()
}
