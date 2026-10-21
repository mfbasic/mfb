//! `platform_family` (`engine/types/types.rs`): the OS family of each registered target.

use crate::codegen::engine::types::{platform_family, PlatformFamily};

#[test]
fn derives_family_for_every_registered_target() {
    // The four strings in `crate::target::NATIVE_BACKENDS`. If a backend is
    // added, its target must gain an arm here (and in `platform_family`).
    assert_eq!(platform_family("linux-x86_64"), PlatformFamily::Linux);
    assert_eq!(platform_family("linux-aarch64"), PlatformFamily::Linux);
    assert_eq!(platform_family("linux-riscv64"), PlatformFamily::Linux);
    assert_eq!(platform_family("macos-aarch64"), PlatformFamily::MacOS);
    // Not yet registered, but the derivation already recognizes it so 47-B
    // can register `windows-x86_64` without touching this function.
    assert_eq!(platform_family("windows-x86_64"), PlatformFamily::Windows);
}

#[test]
#[should_panic(expected = "unregistered target")]
fn panics_on_unregistered_target() {
    let _ = platform_family("plan9-riscv64");
}
