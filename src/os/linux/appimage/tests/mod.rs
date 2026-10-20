use super::*;

/// Every embedded blob, with its recorded length.
fn blobs() -> Vec<(&'static str, &'static [u8], usize)> {
    vec![
        ("x86_64", RUNTIME_X86_64, RUNTIME_X86_64_LEN),
        ("aarch64", RUNTIME_AARCH64, RUNTIME_AARCH64_LEN),
    ]
}

#[test]
fn every_runtime_ends_exactly_at_its_own_length() {
    // §4.2's invariant, and the whole reason `elf_image_end` exists: the seal
    // appends at `blob.len()`, which is only correct while the runtime's
    // ELF-derived end equals it. A future blob with trailing data fails here
    // instead of failing to mount on a user's desktop.
    for (arch, blob, _) in blobs() {
        assert_eq!(
            elf_image_end(blob).unwrap_or_else(|err| panic!("{arch}: {err}")),
            blob.len() as u64,
            "{arch}: the squashfs append point must be the blob's length"
        );
    }
}

#[test]
fn every_runtime_carries_the_appimage_magic() {
    for (arch, blob, _) in blobs() {
        assert_eq!(&blob[0..4], b"\x7fELF", "{arch}: ELF magic");
        // Hex 0x414902 at offset 8, sitting in EI_ABIVERSION and EI_PAD,
        // which the Linux kernel ignores. Upstream `dd`s it in post-strip and
        // the runtime itself never reads it — it exists for `file`,
        // AppImageLauncher, and desktop integration.
        assert_eq!(&blob[8..11], b"AI\x02", "{arch}: AppImage magic");
    }
}

#[test]
fn every_runtime_matches_its_recorded_length() {
    for (arch, blob, len) in blobs() {
        assert_eq!(blob.len(), len, "{arch}: blob length changed");
    }
}

#[test]
fn runtime_for_rejects_an_unpublished_arch() {
    let err = runtime_for("riscv64").expect_err("no riscv64 runtime exists");
    assert!(err.contains("no AppImage runtime is published"), "{err}");
}

#[test]
fn elf_image_end_rejects_a_non_elf_blob() {
    assert!(elf_image_end(&[0u8; 128]).is_err());
    assert!(elf_image_end(b"\x7fELF").is_err(), "too short");
}

#[test]
fn elf_image_end_rejects_a_32_bit_elf() {
    // A well-formed ELF magic and length but EI_CLASS = ELFCLASS32 (1): the
    // runtimes are 64-bit and a 32-bit blob is rejected before header reads.
    let mut blob = vec![0u8; 128];
    blob[0..4].copy_from_slice(b"\x7fELF");
    blob[4] = 1; // ELFCLASS32
    let err = elf_image_end(&blob).unwrap_err();
    assert!(err.contains("not ELF64"), "{err}");
}

/// A blob that clears the ELF64 magic gate but whose section-header table
/// points past its own end drives the `short()` truncation arm: the section
/// loop reads `sh_type`/`sh_offset` out of bounds and reports "truncated".
#[test]
fn elf_image_end_rejects_a_truncated_section_table() {
    // 64-byte header: valid magic + ELFCLASS64, one section header whose
    // offset (0x1000) sits far beyond the blob.
    let mut blob = vec![0u8; 64];
    blob[0..4].copy_from_slice(b"\x7fELF");
    blob[4] = 2; // ELFCLASS64
    blob[0x28..0x30].copy_from_slice(&0x1000u64.to_le_bytes()); // e_shoff
    blob[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
    blob[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes()); // e_shnum
    let err = elf_image_end(&blob).unwrap_err();
    assert!(err.contains("truncated"), "{err}");
}

/// A blob whose section header IS present but whose `sh_offset`/`sh_size`
/// reads run off the end drives the truncation arm one read deeper: `sh_type`
/// resolves, then reading the offset/size at 0x18/0x20 fails.
#[test]
fn elf_image_end_rejects_a_truncated_section_body() {
    // The single section header starts at 0x40 (right after the Ehdr) and is
    // only 8 bytes long, so sh_type (at +0x04) reads but sh_offset (+0x18)
    // runs past the blob.
    let mut blob = vec![0u8; 0x48];
    blob[0..4].copy_from_slice(b"\x7fELF");
    blob[4] = 2; // ELFCLASS64
    blob[0x28..0x30].copy_from_slice(&0x40u64.to_le_bytes()); // e_shoff
    blob[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
    blob[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes()); // e_shnum
                                                           // sh_type at 0x44 stays 0 (not SHT_NOBITS), so the loop proceeds to read
                                                           // sh_offset at 0x40+0x18 = 0x58, which is past the 0x48-byte blob.
    let err = elf_image_end(&blob).unwrap_err();
    assert!(err.contains("truncated"), "{err}");
}

/// A section marked `SHT_NOBITS` (type 8) contributes no file extent, so it is
/// skipped; the image end falls back to the section-header table's own end.
#[test]
fn elf_image_end_skips_nobits_sections() {
    let mut blob = vec![0u8; 0x80];
    blob[0..4].copy_from_slice(b"\x7fELF");
    blob[4] = 2; // ELFCLASS64
    blob[0x28..0x30].copy_from_slice(&0x40u64.to_le_bytes()); // e_shoff
    blob[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
    blob[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes()); // e_shnum
    blob[0x44..0x48].copy_from_slice(&8u32.to_le_bytes()); // sh_type = SHT_NOBITS
                                                           // No section extents count, so the end is sht_end = 0x40 + 64*1 = 0x80.
    assert_eq!(elf_image_end(&blob).unwrap(), 0x80);
}

#[test]
#[cfg(unix)]
fn read_dir_node_rejects_a_non_file_non_dir_non_symlink() {
    // A FIFO is neither a file, a directory, nor a symlink, so the reader
    // rejects it rather than silently dropping or misclassifying it.
    let dir = tempfile::tempdir().unwrap();
    let appdir = dir.path().join(BUILD_DIR).join("hello-glibc.AppDir");
    fs::create_dir_all(&appdir).unwrap();
    let fifo = appdir.join("a_pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo failed");
    let err = read_appdir(&appdir).expect_err("a FIFO cannot be represented");
    assert!(
        err.contains("neither a file, a directory, nor a symlink"),
        "{err}"
    );
}

#[test]
fn remove_appdir_reports_a_hard_failure() {
    // A non-NotFound error (here: the AppDir path is a regular file, so
    // `remove_dir_all` fails with ENOTDIR) surfaces as a diagnostic, not a
    // silent success.
    let dir = tempfile::tempdir().unwrap();
    let build = dir.path().join(BUILD_DIR);
    fs::create_dir_all(&build).unwrap();
    let path = build.join(crate::os::linux::appdir::appdir_name("hello", "glibc"));
    fs::write(&path, b"not a directory").unwrap();
    let err = remove_appdir(dir.path(), "hello", "glibc").expect_err("path is a file");
    assert!(err.contains("failed to remove"), "{err}");
}

/// Build a small AppDir on disk, matching plan-51-A's shape closely enough to
/// exercise every node type the reader has to handle.
fn fixture_appdir(root: &Path) -> PathBuf {
    let appdir = root.join(BUILD_DIR).join("hello-glibc.AppDir");
    fs::create_dir_all(appdir.join("usr/bin")).unwrap();
    fs::create_dir_all(appdir.join("usr/share/applications")).unwrap();
    fs::write(appdir.join("usr/bin/hello"), b"\x7fELF fake").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            appdir.join("usr/bin/hello"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fs::write(appdir.join("hello.desktop"), b"[Desktop Entry]\n").unwrap();
    fs::write(appdir.join("hello.png"), b"fake png").unwrap();
    crate::os::linux::appdir::make_symlink_for_test("usr/bin/hello", &appdir.join("AppRun"));
    crate::os::linux::appdir::make_symlink_for_test("hello.png", &appdir.join(".DirIcon"));
    appdir
}

#[test]
fn read_appdir_keeps_symlinks_as_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let appdir = fixture_appdir(dir.path());
    let tree = read_appdir(&appdir).expect("read");
    let SquashNode::Dir { entries, .. } = &tree.root else {
        panic!("root must be a directory");
    };
    assert_eq!(
        entries.get("AppRun"),
        Some(&SquashNode::Symlink {
            target: "usr/bin/hello".to_string()
        }),
        "AppRun must stay a symlink, not become a second copy of the ELF"
    );
    assert_eq!(
        entries.get(".DirIcon"),
        Some(&SquashNode::Symlink {
            target: "hello.png".to_string()
        })
    );
}

#[test]
fn read_appdir_preserves_the_executable_bit() {
    let dir = tempfile::tempdir().unwrap();
    let appdir = fixture_appdir(dir.path());
    let tree = read_appdir(&appdir).expect("read");
    let SquashNode::Dir { entries, .. } = &tree.root else {
        panic!()
    };
    let SquashNode::Dir { entries: usr, .. } = entries.get("usr").expect("usr") else {
        panic!()
    };
    let SquashNode::Dir { entries: bin, .. } = usr.get("bin").expect("bin") else {
        panic!()
    };
    match bin.get("hello").expect("the executable") {
        SquashNode::File { data, mode } => {
            assert_eq!(data, b"\x7fELF fake");
            assert_eq!(*mode, 0o755, "the ELF's 0755 must survive into the image");
        }
        other => panic!("expected a file, got {other:?}"),
    }
}

#[test]
fn read_appdir_omits_usr_lib_when_the_build_vendors_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let appdir = fixture_appdir(dir.path());
    let tree = read_appdir(&appdir).expect("read");
    let SquashNode::Dir { entries, .. } = &tree.root else {
        panic!()
    };
    let SquashNode::Dir { entries: usr, .. } = entries.get("usr").expect("usr") else {
        panic!()
    };
    assert!(!usr.contains_key("lib"));

    // And it appears once vendoring puts something there.
    fs::create_dir_all(appdir.join("usr/lib")).unwrap();
    fs::write(appdir.join("usr/lib/libthing.so"), b"blob").unwrap();
    let tree = read_appdir(&appdir).expect("read");
    let SquashNode::Dir { entries, .. } = &tree.root else {
        panic!()
    };
    let SquashNode::Dir { entries: usr, .. } = entries.get("usr").expect("usr") else {
        panic!()
    };
    assert!(usr.contains_key("lib"));
}

#[cfg(unix)]
#[test]
fn seal_concatenates_the_runtime_and_a_valid_squashfs() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    fixture_appdir(dir.path());
    let path = seal(dir.path(), "hello", "glibc", "x86_64").expect("seal");
    assert_eq!(path, dir.path().join("build").join("hello-glibc.AppImage"));
    let sealed = fs::read(&path).unwrap();

    // The first runtime.len() bytes are the blob, byte-for-byte as published.
    assert_eq!(&sealed[..RUNTIME_X86_64.len()], RUNTIME_X86_64);
    // And a valid squashfs superblock begins at exactly that offset, with no
    // padding — padding would be read as the superblock and fail the mount.
    assert_eq!(
        &sealed[RUNTIME_X86_64.len()..RUNTIME_X86_64.len() + 4],
        b"hsqs"
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755,
        "an AppImage without the executable bit gets no diagnostic beyond \
             'Permission denied'"
    );
}

#[test]
fn seal_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    fixture_appdir(dir.path());
    let first = fs::read(seal(dir.path(), "hello", "glibc", "x86_64").expect("seal")).unwrap();
    let second = fs::read(seal(dir.path(), "hello", "glibc", "x86_64").expect("seal")).unwrap();
    assert_eq!(first, second, "two seals of one AppDir must be identical");
}

#[test]
fn seal_uses_the_arch_specific_runtime() {
    let dir = tempfile::tempdir().unwrap();
    fixture_appdir(dir.path());
    let arm = fs::read(seal(dir.path(), "hello", "glibc", "aarch64").expect("seal")).unwrap();
    assert_eq!(&arm[..RUNTIME_AARCH64.len()], RUNTIME_AARCH64);
}

#[test]
fn seal_reports_a_missing_appdir() {
    let dir = tempfile::tempdir().unwrap();
    let err = seal(dir.path(), "hello", "glibc", "x86_64").expect_err("no AppDir");
    assert!(err.contains("does not exist"), "{err}");
}

#[test]
fn remove_appdir_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    fixture_appdir(dir.path());
    remove_appdir(dir.path(), "hello", "glibc").expect("first remove");
    assert!(!dir.path().join("build/hello-glibc.AppDir").exists());
    remove_appdir(dir.path(), "hello", "glibc").expect("removing an absent AppDir is not an error");
}
