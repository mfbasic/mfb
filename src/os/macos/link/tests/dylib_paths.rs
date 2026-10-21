//! Every library the macOS backend imports from maps to the install name dyld
//! will load, and the versioned framework paths match the object plan's.

use super::*;

/// A wrong install name is a binary that dies at launch with "Library not loaded",
/// so every logical name the plans can produce is pinned to its path. Frameworks
/// that dyld resolves through `Versions/A` carry it; the older ones do not.
#[test]
fn each_logical_library_maps_to_its_install_name() {
    for (library, path) in [
        ("libSystem", "/usr/lib/libSystem.B.dylib"),
        ("libobjc", "/usr/lib/libobjc.A.dylib"),
        ("libz", "/usr/lib/libz.1.dylib"),
        (
            "Network",
            "/System/Library/Frameworks/Network.framework/Network",
        ),
        (
            "AudioToolbox",
            "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox",
        ),
        (
            "CoreAudio",
            "/System/Library/Frameworks/CoreAudio.framework/CoreAudio",
        ),
        (
            "CoreFoundation",
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
        ),
        (
            "AppKit",
            "/System/Library/Frameworks/AppKit.framework/AppKit",
        ),
        (
            "Foundation",
            "/System/Library/Frameworks/Foundation.framework/Foundation",
        ),
        (
            "Metal",
            "/System/Library/Frameworks/Metal.framework/Versions/A/Metal",
        ),
        (
            "QuartzCore",
            "/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore",
        ),
        (
            "CoreGraphics",
            "/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics",
        ),
        (
            "CoreText",
            "/System/Library/Frameworks/CoreText.framework/Versions/A/CoreText",
        ),
    ] {
        assert_eq!(dylib_path(library).as_deref(), Ok(path), "{library}");
    }
}
