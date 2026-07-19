//! AppImage sealing (plan-51-C): join a plan-51-A AppDir and a plan-51-B
//! squashfs image into a single double-clickable `build/<name>.AppImage`.
//!
//! An AppImage is `[runtime ELF][squashfs image]` concatenated at exactly the
//! runtime's length. There is no container header, no alignment, and no index —
//! the runtime computes the boundary from its own ELF headers at startup and
//! hands the offset to its bundled squashfuse. So the "seal" is genuinely a
//! concatenation plus a `chmod +x`; the engineering is in *where* it happens in
//! the build pipeline (§3.2), because a sealed artifact cannot gain files after
//! it closes.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::os::linux::squashfs::{self, SquashNode, SquashTree};
use crate::os::BUILD_DIR;

/// The upstream release tag the embedded runtimes come from.
///
/// Pinned, deliberately not `continuous`: `continuous` is a rolling tag, which
/// would make AppImages non-reproducible and turn an upstream push into a silent
/// change in our output. Bumping this is a deliberate commit with a re-verified
/// signature.
const RUNTIME_TAG: &str = "20251108";

/// The AppImage type-2 runtime for x86-64, from release tag [`RUNTIME_TAG`].
/// sha256 `2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d`;
/// the upstream GPG signature was verified against `signing-pubkey.asc`
/// (EDDSA key `570C77ACEA40C0F1B758902CBF96CCA56490F695`) before committing.
const RUNTIME_X86_64: &[u8] = include_bytes!("runtime-x86_64");
/// The AppImage type-2 runtime for aarch64, from release tag [`RUNTIME_TAG`].
/// sha256 `00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444`;
/// signature verified against the same key.
const RUNTIME_AARCH64: &[u8] = include_bytes!("runtime-aarch64");

/// Recorded blob lengths. A stale or truncated copy fails a unit test rather
/// than shipping.
///
/// Test-only by construction: the guard they serve is
/// `every_runtime_ends_exactly_at_its_own_length`, and nothing in the shipping
/// build should consult a hardcoded length instead of the blob itself. The
/// `#[cfg(test)]` keeps them honest rather than deleting a supply-chain check to
/// quiet a dead-code warning (bug-326-D7).
#[cfg(test)]
const RUNTIME_X86_64_LEN: usize = 944_632;
#[cfg(test)]
const RUNTIME_AARCH64_LEN: usize = 936_456;

/// The AppImage type-2 runtime for `arch` (plan-51-C §4.3).
///
/// Embedded rather than downloaded so `mfb build` stays hermetic and offline —
/// the same reason this compiler has a built-in linker. Cross-building Linux
/// from macOS is the primary workflow, so these cannot be `#[cfg]`-gated to
/// Linux hosts: the macOS build is precisely the one that needs them.
///
/// Each blob is a static-PIE musl binary bundling libfuse 3.15.0, squashfuse,
/// libzstd, and zlib. **libfuse2 is not required on the host** — a common
/// misconception from older AppImages. What *is* required is a setuid
/// `fusermount`/`fusermount3` on `$PATH` plus `/dev/fuse`.
fn runtime_for(arch: &str) -> Result<&'static [u8], String> {
    match arch {
        "x86_64" => Ok(RUNTIME_X86_64),
        "aarch64" => Ok(RUNTIME_AARCH64),
        // riscv64 reaches here only through a bug: app mode is unsupported there
        // (plan-51-A §3.3), and AppImage/type2-runtime publishes no riscv64
        // runtime to seal one with even if it were.
        other => Err(format!(
            "no AppImage runtime is published for {other}; app mode supports \
             linux-x86_64 and linux-aarch64 only"
        )),
    }
}

/// The offset the AppImage runtime will look for the squashfs at (plan-51-C
/// §4.2): the end of its own ELF, computed exactly as upstream's
/// `runtime.c:247-277` does.
///
/// ```c
/// sht_end = ehdr.e_shoff + (ehdr.e_shentsize * ehdr.e_shnum);
/// last_section_end = shdr.sh_offset + shdr.sh_size;
/// return sht_end > last_section_end ? sht_end : last_section_end;
/// ```
///
/// For every runtime upstream has published this equals the blob's length, so
/// [`seal`] appends at `blob.len()`. That is a property of how upstream *links*
/// the blob, not of the format — a future blob with trailing data would put our
/// squashfs somewhere the runtime does not look, and the only symptom would be a
/// mount failure on a user's desktop. This is the computation that turns a blob
/// swap into a failing unit test.
fn elf_image_end(runtime: &[u8]) -> Result<u64, String> {
    let short = || "AppImage runtime is truncated".to_string();
    if runtime.len() < 64 || &runtime[0..4] != b"\x7fELF" {
        return Err("AppImage runtime is not an ELF image".to_string());
    }
    if runtime[4] != 2 {
        return Err("AppImage runtime is not ELF64".to_string());
    }
    let read_u16 = |at: usize| -> Result<u64, String> {
        runtime
            .get(at..at + 2)
            .map(|slice| u16::from_le_bytes(slice.try_into().expect("2 bytes")) as u64)
            .ok_or_else(short)
    };
    let read_u64 = |at: usize| -> Result<u64, String> {
        runtime
            .get(at..at + 8)
            .map(|slice| u64::from_le_bytes(slice.try_into().expect("8 bytes")))
            .ok_or_else(short)
    };
    // Elf64_Ehdr: e_shoff at 0x28, e_shentsize at 0x3A, e_shnum at 0x3C.
    let shoff = read_u64(0x28)?;
    let shentsize = read_u16(0x3A)?;
    let shnum = read_u16(0x3C)?;
    let sht_end = shoff + shentsize * shnum;

    let mut last_section_end = 0u64;
    for index in 0..shnum {
        let header = (shoff + index * shentsize) as usize;
        // Elf64_Shdr: sh_type at 0x04, sh_offset at 0x18, sh_size at 0x20.
        let sh_type = runtime
            .get(header + 4..header + 8)
            .map(|slice| u32::from_le_bytes(slice.try_into().expect("4 bytes")))
            .ok_or_else(short)?;
        // SHT_NOBITS (8) occupies no file space, so its sh_offset/sh_size say
        // nothing about where the image ends.
        if sh_type == 8 {
            continue;
        }
        let end = read_u64(header + 0x18)? + read_u64(header + 0x20)?;
        last_section_end = last_section_end.max(end);
    }
    Ok(sht_end.max(last_section_end))
}

/// Seal an AppDir into a single-file AppImage (plan-51-C §4.1):
/// `[runtime ELF][squashfs]`, concatenated at the runtime's exact length.
///
/// The output is `chmod +x`: an AppImage without the executable bit is a file the
/// user cannot run and gets no diagnostic from beyond "Permission denied".
///
/// Deterministic — the runtime is a fixed blob and the squashfs sets
/// `mkfs_time`/`mtime` to 0 (plan-51-B §4.2), so two builds of one project are
/// byte-identical.
///
/// **Alignment: none, and padding is actively wrong.** The published
/// `runtime-x86_64` is 944632 bytes (`% 4096 == 2552`), deliberately unaligned.
/// Any padding between the runtime and the squashfs would be read as the
/// superblock and fail the mount.
pub(crate) fn seal(
    project_dir: &Path,
    project_name: &str,
    flavor_suffix: &str,
    arch: &str,
) -> Result<PathBuf, String> {
    let build_dir = project_dir.join(BUILD_DIR);
    let appdir = build_dir.join(crate::os::linux::appdir::appdir_name(
        project_name,
        flavor_suffix,
    ));
    if !appdir.is_dir() {
        return Err(format!(
            "cannot seal an AppImage: '{}' does not exist",
            appdir.display()
        ));
    }
    let tree = read_appdir(&appdir)?;
    let image = squashfs::write(&tree)?;

    let runtime = runtime_for(arch)?;
    let end = elf_image_end(runtime)?;
    if end != runtime.len() as u64 {
        return Err(format!(
            "the embedded AppImage runtime for {arch} (release {RUNTIME_TAG}) ends at {end} but \
             is {} bytes long; the squashfs would be appended where the runtime does not look \
             for it",
            runtime.len()
        ));
    }

    let mut sealed = Vec::with_capacity(runtime.len() + image.len());
    sealed.extend_from_slice(runtime);
    sealed.extend_from_slice(&image);

    let path = build_dir.join(format!("{project_name}-{flavor_suffix}.AppImage"));
    fs::write(&path, &sealed)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    let mut permissions = fs::metadata(&path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions)
        .map_err(|err| format!("failed to mark '{}' executable: {err}", path.display()))?;
    Ok(path)
}

/// Remove the intermediate AppDir once the seal has consumed it (plan-51-C
/// §3.3). `--app` emits one artifact, matching macOS `--app`'s single `.app`;
/// `--app-debug` keeps this directory for the case where you need to see what
/// went in.
pub(crate) fn remove_appdir(
    project_dir: &Path,
    project_name: &str,
    flavor_suffix: &str,
) -> Result<(), String> {
    let appdir = project_dir
        .join(BUILD_DIR)
        .join(crate::os::linux::appdir::appdir_name(
            project_name,
            flavor_suffix,
        ));
    match fs::remove_dir_all(&appdir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to remove '{}': {err}", appdir.display())),
    }
}

/// Walk an on-disk AppDir into a [`SquashTree`] (plan-51-C §4.4).
///
/// ⚠️ **This is the one place a symlink must not be followed.** `fs::metadata`
/// follows; `fs::symlink_metadata` does not. Getting it wrong turns `AppRun` into
/// a second copy of the ELF — which still *works*, silently, at double the size.
fn read_appdir(path: &Path) -> Result<SquashTree, String> {
    Ok(SquashTree {
        root: read_dir_node(path)?,
    })
}

fn read_dir_node(path: &Path) -> Result<SquashNode, String> {
    let mut entries = BTreeMap::new();
    let listing =
        fs::read_dir(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    for entry in listing {
        let entry = entry.map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
        let child = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("'{}' has a non-UTF-8 name", child.display()))?;
        let metadata = fs::symlink_metadata(&child)
            .map_err(|err| format!("failed to stat '{}': {err}", child.display()))?;
        let node = if metadata.file_type().is_symlink() {
            let target = fs::read_link(&child)
                .map_err(|err| format!("failed to read link '{}': {err}", child.display()))?;
            SquashNode::Symlink {
                target: target
                    .to_str()
                    .ok_or_else(|| format!("'{}' points at a non-UTF-8 path", child.display()))?
                    .to_string(),
            }
        } else if metadata.is_dir() {
            read_dir_node(&child)?
        } else if metadata.is_file() {
            SquashNode::File {
                data: fs::read(&child)
                    .map_err(|err| format!("failed to read '{}': {err}", child.display()))?,
                mode: (metadata.permissions().mode() & 0o7777) as u16,
            }
        } else {
            // A device node, FIFO, or socket cannot appear in an AppDir the
            // compiler wrote, and `SquashNode` deliberately cannot represent one.
            return Err(format!(
                "'{}' is neither a file, a directory, nor a symlink",
                child.display()
            ));
        };
        entries.insert(name, node);
    }
    let mode = (fs::symlink_metadata(path)
        .map_err(|err| format!("failed to stat '{}': {err}", path.display()))?
        .permissions()
        .mode()
        & 0o7777) as u16;
    Ok(SquashNode::Dir { entries, mode })
}

#[cfg(test)]
mod tests {
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

    /// Build a small AppDir on disk, matching plan-51-A's shape closely enough to
    /// exercise every node type the reader has to handle.
    fn fixture_appdir(root: &Path) -> PathBuf {
        let appdir = root.join(BUILD_DIR).join("hello-glibc.AppDir");
        fs::create_dir_all(appdir.join("usr/bin")).unwrap();
        fs::create_dir_all(appdir.join("usr/share/applications")).unwrap();
        fs::write(appdir.join("usr/bin/hello"), b"\x7fELF fake").unwrap();
        fs::set_permissions(
            appdir.join("usr/bin/hello"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::write(appdir.join("hello.desktop"), b"[Desktop Entry]\n").unwrap();
        fs::write(appdir.join("hello.png"), b"fake png").unwrap();
        std::os::unix::fs::symlink("usr/bin/hello", appdir.join("AppRun")).unwrap();
        std::os::unix::fs::symlink("hello.png", appdir.join(".DirIcon")).unwrap();
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

    #[test]
    fn seal_concatenates_the_runtime_and_a_valid_squashfs() {
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
        remove_appdir(dir.path(), "hello", "glibc")
            .expect("removing an absent AppDir is not an error");
    }
}
