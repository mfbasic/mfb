//! Every Linux backend writes a real ELF, from this process, on this machine.
//!
//! `NativeBackend::write_executable` is the end of the build: it lowers the
//! module, plans and regallocs it, emits native code, encodes an image and
//! writes one executable per libc world. The three Linux backends' copies of it
//! — `linux_x86_64`, `linux_aarch64`, `linux_riscv64` — sat at 24.77%, 24.65%
//! and 36.41%, the widest untested functions in the tree.
//!
//! They were assumed untestable in process: writing an executable sounds like
//! spawning a linker. For these three it is not. MFB emits its own ELF
//! (`os::linux::link::elf`), and there is no `Command::new` anywhere under
//! `src/os/linux/` outside the AppImage sealer — so a Linux executable can be
//! produced on a macOS host with nothing but `fs::write`, which is exactly what
//! cross-compilation means and what CI's five-platform matrix depends on.
//!
//! What these assert beyond "it ran": that the bytes are an ELF64 for the arch
//! that was asked for. A backend wired to the wrong encoder — or an arch token
//! that drifted from the `e_machine` the encoder writes — produces a file of
//! the right size, in the right place, with the right name, that no Linux
//! kernel will exec. The file existing proves nothing; its header does.

use super::*;
use crate::ir::IrProject;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::NativeBuildMode;

/// A program with enough in it to reach the parts of the pipeline that differ
/// per architecture — arithmetic, a call, a string, a collection — rather than
/// an empty `main`, which would exercise little more than the entry stub.
const SRC: &str = "\
IMPORT io

FUNC total(values AS List OF Integer) AS Integer
  MUT sum AS Integer = 0
  FOR EACH v IN values
    sum = sum + v * 2
  NEXT
  RETURN sum
END FUNC

FUNC main() AS Integer
  LET values AS List OF Integer = [1, 2, 3, 4]
  io::print(\"total=\" & toString(total(values)))
  RETURN 0
END FUNC
";

/// `e_machine` (ELF header bytes 18..20, little-endian) per arch token, from
/// the ELF psABI. Written out rather than read back from the encoder, so the
/// two have to agree with something outside themselves.
const E_MACHINE: &[(&str, &str, u16)] = &[
    ("linux", "x86_64", 0x3E),
    ("linux", "aarch64", 0xB7),
    ("linux", "riscv64", 0xF3),
];

/// A scratch project directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mfb_cross_{tag}_{nonce}"));
        std::fs::create_dir_all(&dir).expect("create scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every Linux backend writes one ELF per libc flavor, and each carries the
/// `e_machine` of the arch it was built for.
///
/// One test over the three because the interesting failure is a mix-up between
/// them: three separate tests that each checked "an ELF appeared" would all
/// pass with two backends wired to the same encoder.
#[test]
fn every_linux_backend_writes_an_elf_for_its_own_architecture() {
    for &(os, arch, machine) in E_MACHINE {
        let scratch = Scratch::new(arch);
        let ir = crate::testutil::named_ir_for_src(SRC, "crossprog");
        let backend = backend_for(&BuildTarget {
            os: os.to_string(),
            arch: arch.to_string(),
        })
        .unwrap_or_else(|err| panic!("{os}-{arch}: {err}"));

        let written = backend
            .write_executable(
                &scratch.0,
                &ir,
                &[],
                None,
                NativeBuildMode::Console,
                None,
                None,
                false,
                None,
                &|_| {},
            )
            .unwrap_or_else(|err| panic!("{os}-{arch}: write_executable: {err}"));

        assert_eq!(
            written.len(),
            LinuxFlavor::ALL.len(),
            "{os}-{arch} must write one executable per libc world, got {written:?}"
        );

        for (path, flavor) in written.iter().zip(LinuxFlavor::ALL) {
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some(format!("crossprog-{}.out", flavor.suffix()).as_str()),
                "{os}-{arch}: the artifact name carries the libc world"
            );
            let bytes = std::fs::read(path)
                .unwrap_or_else(|err| panic!("{os}-{arch}: read {}: {err}", path.display()));
            assert!(
                bytes.len() > 64,
                "{os}-{arch}/{}: an ELF is at least its 64-byte header, got {} bytes",
                flavor.suffix(),
                bytes.len()
            );
            assert_eq!(
                &bytes[0..4],
                b"\x7fELF",
                "{os}-{arch}/{}: not an ELF",
                flavor.suffix()
            );
            assert_eq!(
                (bytes[4], bytes[5]),
                (2, 1),
                "{os}-{arch}/{}: every target here is 64-bit little-endian",
                flavor.suffix()
            );
            assert_eq!(
                u16::from_le_bytes([bytes[18], bytes[19]]),
                machine,
                "{os}-{arch}/{}: e_machine names the architecture the kernel \
                 will refuse to exec if it is wrong",
                flavor.suffix()
            );
        }
    }
}

/// The two libc worlds are not the same file.
///
/// They come off the same encoder from the same image, so a backend that
/// ignored `flavor` would write the identical bytes twice under two names and
/// every assertion above would still hold. What differs is the interpreter and
/// the runpath — glibc and musl load from different places — so the bytes must
/// not match.
#[test]
fn the_two_libc_flavors_are_different_binaries() {
    let scratch = Scratch::new("flavors");
    let ir = crate::testutil::named_ir_for_src(SRC, "crossprog");
    let backend = backend_for(&BuildTarget {
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
    })
    .expect("linux-x86_64 is registered");

    let written = backend
        .write_executable(
            &scratch.0,
            &ir,
            &[],
            None,
            NativeBuildMode::Console,
            None,
            None,
            false,
            None,
            &|_| {},
        )
        .expect("write_executable");

    let bytes: Vec<Vec<u8>> = written
        .iter()
        .map(|path| std::fs::read(path).expect("read artifact"))
        .collect();
    assert_ne!(
        bytes[0], bytes[1],
        "glibc and musl executables must differ — the same bytes under two \
         names means the flavor was never consulted"
    );
}

/// An `-app` build writes an AppDir per libc world instead of a bare
/// executable, and `finalize_app_bundle` seals each into an AppImage.
///
/// This is the second half of every Linux backend's file and it forks off the
/// same `write_executable`: `build_mode.is_app()` picks `write_linked_appdir`
/// over `write_linked_executable`, and the manifest version becomes the
/// AppImage's `X-AppImage-Version`. The version is not optional there —
/// `app_version.ok_or("internal error: app mode requires the manifest
/// version")` — so the two directions are both asserted, because a backend that
/// silently defaulted it would ship every AppImage claiming the same version.
///
/// riscv64 is excluded, and not for convenience: `supports_app_mode()` is false
/// for it, GTK4 having never been ported, and `-app` is rejected at the CLI.
#[test]
fn an_app_build_writes_an_appdir_per_libc_world_and_seals_it() {
    for arch in ["x86_64", "aarch64"] {
        let scratch = Scratch::new(&format!("app_{arch}"));
        let ir = crate::testutil::named_ir_for_src(SRC, "crossapp");
        let backend = backend_for(&BuildTarget {
            os: "linux".to_string(),
            arch: arch.to_string(),
        })
        .unwrap_or_else(|err| panic!("linux-{arch}: {err}"));
        assert!(
            backend.supports_app_mode(),
            "linux-{arch} is one of the app-capable backends"
        );

        let missing_version = backend.write_executable(
            &scratch.0,
            &ir,
            &[],
            None,
            NativeBuildMode::LinuxApp,
            None,
            None,
            false,
            None,
            &|_| {},
        );
        assert!(
            missing_version
                .as_ref()
                .err()
                .is_some_and(|err| err.contains("app mode requires the manifest version")),
            "linux-{arch}: an app build with no version must be refused, got \
             {missing_version:?}"
        );

        let written = backend
            .write_executable(
                &scratch.0,
                &ir,
                &[],
                None,
                NativeBuildMode::LinuxApp,
                None,
                Some("1.2.3"),
                false,
                None,
                &|_| {},
            )
            .unwrap_or_else(|err| panic!("linux-{arch}: app write_executable: {err}"));

        assert_eq!(written.len(), LinuxFlavor::ALL.len());
        for (path, flavor) in written.iter().zip(LinuxFlavor::ALL) {
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some(format!("crossapp-{}.AppDir", flavor.suffix()).as_str()),
                "linux-{arch}: an app build produces an AppDir, not an executable"
            );
            assert!(
                path.is_dir(),
                "linux-{arch}/{}: the AppDir is a directory",
                flavor.suffix()
            );
        }

        let sealed = backend
            .finalize_app_bundle(&scratch.0, "crossapp", false)
            .unwrap_or_else(|err| panic!("linux-{arch}: finalize_app_bundle: {err}"));
        assert_eq!(sealed.len(), LinuxFlavor::ALL.len());
        for (path, flavor) in sealed.iter().zip(LinuxFlavor::ALL) {
            assert!(
                path.is_file(),
                "linux-{arch}/{}: the seal produces one file: {}",
                flavor.suffix(),
                path.display()
            );
            let appdir = path.with_extension("AppDir");
            assert!(
                !appdir.exists(),
                "linux-{arch}/{}: the intermediate AppDir is removed unless \
                 --app-debug asked to keep it: {}",
                flavor.suffix(),
                appdir.display()
            );
        }
    }
}

/// `write_executable` reports progress, in order, and the caller's callback is
/// the only channel it has.
///
/// The CLI's `--verbose` build log is this sequence; a stage that stopped
/// announcing itself would leave a long silence in the middle of a build with
/// no other symptom. Asserting the phases rather than a count keeps it a
/// statement about the pipeline — lower, plan, emit, encode, link — and not
/// about how many strings happen to be passed.
#[test]
fn write_executable_announces_each_stage() {
    let scratch = Scratch::new("progress");
    let ir = crate::testutil::named_ir_for_src(SRC, "crossprog");
    let backend = backend_for(&BuildTarget {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
    })
    .expect("linux-aarch64 is registered");

    let seen = std::sync::Mutex::new(Vec::<String>::new());
    backend
        .write_executable(
            &scratch.0,
            &ir,
            &[],
            None,
            NativeBuildMode::Console,
            None,
            None,
            false,
            None,
            &|stage| seen.lock().expect("progress lock").push(stage.to_string()),
        )
        .expect("write_executable");

    let seen = seen.into_inner().expect("progress lock");
    assert_eq!(
        seen.first().map(String::as_str),
        Some("lowering module"),
        "lowering is announced once, before the per-flavor loop: {seen:?}"
    );
    for stage in [
        "planning + regalloc",
        "emitting native code",
        "encoding image",
    ] {
        assert_eq!(
            seen.iter().filter(|s| s.as_str() == stage).count(),
            LinuxFlavor::ALL.len(),
            "{stage:?} runs once per libc world: {seen:?}"
        );
    }
}

/// Every diagnostic dump, on every registered target, through the dispatchers
/// the CLI calls.
///
/// `-nir`, `-nplan`, `-nobj`, `-ncode` and `-mir` are the compiler's own
/// windows into what it is about to emit, and they are what an ABI or codegen
/// question gets answered with. Nothing in process had ever called them: the
/// five module-level dispatchers in `src/target.rs`, each backend's five
/// forwarders, and the five shared writers in `target/linux_common/mod.rs` were
/// all unexecuted, and `linux_common/mod.rs` sat at 22.62% because of it.
///
/// They go through `crate::target::write_*` rather than the backend directly,
/// because the dispatcher is where the CAPABILITY gate lives — "native
/// executable output does not support X yet" — and a test that skipped it would
/// leave the refusal untested along with the gate that produces it.
///
/// The assertion is that each dump lands at `<name>.<ext>` and is non-empty.
/// Which is not much on its own, and is not meant to be: the CONTENT of the NIR
/// and plan dumps is asserted by `nir_json.rs`, `nir_validation.rs` and
/// `plan_validation.rs`. What this pins is the plumbing — the extension, the
/// destination, and that a target advertising a capability can actually deliver
/// it. A backend whose forwarder wrote to the wrong path, or whose capability
/// flag disagreed with its implementation, is invisible to every one of those
/// other suites.
#[test]
fn every_dump_kind_writes_on_every_target_that_advertises_it() {
    use crate::target::{
        write_mir, write_native_code_plan, write_native_object_plan, write_native_plan, write_nir,
    };

    type Writer =
        fn(&Path, &IrProject, &BuildTarget, &[PathBuf], NativeBuildMode) -> Result<PathBuf, String>;

    // (extension, writer, the capability that gates it)
    let kinds: &[(&str, Writer, fn(&BackendCapabilities) -> bool)] = &[
        ("nir", write_nir, |c| c.native_ir),
        ("nplan", write_native_plan, |c| c.native_plan),
        ("nobj", write_native_object_plan, |c| c.native_object_plan),
        ("ncode", write_native_code_plan, |c| c.native_code_plan),
        // `-mir` shares the code-plan capability: same lowering, captured
        // before register allocation and instruction selection.
        ("mir", write_mir, |c| c.native_code_plan),
    ];

    for target in registered_targets() {
        let capabilities = backend_for(&target)
            .unwrap_or_else(|err| panic!("{}: {err}", target.name()))
            .capabilities();
        for (extension, write, gated_on) in kinds {
            let scratch = Scratch::new(&format!("dump_{}_{extension}", target.arch));
            let ir = crate::testutil::named_ir_for_src(SRC, "dumpprog");
            let written = write(&scratch.0, &ir, &target, &[], NativeBuildMode::Console);

            if !gated_on(&capabilities) {
                // A target that does not advertise the capability must be
                // refused by the dispatcher, not attempted by the backend.
                assert!(
                    written
                        .as_ref()
                        .err()
                        .is_some_and(|err| err.contains("does not support")),
                    "{} advertises no {extension} capability, so the dispatcher \
                     must refuse it rather than call a backend that cannot \
                     deliver; it said {written:?}",
                    target.name()
                );
                continue;
            }

            let path =
                written.unwrap_or_else(|err| panic!("{} -{extension}: {err}", target.name()));
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some(format!("dumpprog.{extension}").as_str()),
                "{} -{extension}: the dump is named after the project",
                target.name()
            );
            let bytes = std::fs::metadata(&path)
                .unwrap_or_else(|err| {
                    panic!("{} -{extension}: {}: {err}", target.name(), path.display())
                })
                .len();
            assert!(
                bytes > 0,
                "{} -{extension}: an empty dump is a writer that ran and \
                 described nothing",
                target.name()
            );
        }
    }
}

/// Every target that advertises `executable` writes one, through the
/// dispatcher, with the right image magic for its OS.
///
/// The Linux tests above call the backend directly, which is right for what
/// they assert (one artifact per libc world, `e_machine`, the flavors
/// differing). This one goes through `crate::target::write_executable`, and
/// covers the two backends the Linux tests cannot reach at all: macOS, which
/// writes a Mach-O and signs it — in process, with no `codesign` — and Windows,
/// which writes a PE.
///
/// The magic number is the assertion because it is the one thing a loader reads
/// first and a build cannot fake. A backend wired to the wrong encoder writes a
/// file of a plausible size, in the right place, under the right name, that no
/// kernel on any of the three systems will exec.
#[test]
fn every_executable_target_writes_an_image_its_os_can_load() {
    // (os, the first bytes of a loadable image on it)
    const MAGIC: &[(&str, &[u8])] = &[
        // Mach-O 64-bit, little-endian: 0xFEEDFACF.
        ("macos", &[0xCF, 0xFA, 0xED, 0xFE]),
        ("linux", b"\x7fELF"),
        // A PE opens with the DOS stub's "MZ"; the PE header itself is at the
        // offset that stub points at.
        ("windows", b"MZ"),
    ];

    for target in registered_targets() {
        let capabilities = backend_for(&target)
            .unwrap_or_else(|err| panic!("{}: {err}", target.name()))
            .capabilities();
        if !capabilities.executable {
            continue;
        }
        let (_, magic) = MAGIC
            .iter()
            .find(|(os, _)| *os == target.os)
            .unwrap_or_else(|| panic!("{}: no image magic recorded for this OS", target.name()));

        let scratch = Scratch::new(&format!("exe_{}_{}", target.os, target.arch));
        let ir = crate::testutil::named_ir_for_src(SRC, "exeprog");
        let written = crate::target::write_executable(
            &scratch.0,
            &ir,
            &target,
            &[],
            None,
            NativeBuildMode::Console,
            None,
            None,
            false,
            None,
            &|_| {},
        )
        .unwrap_or_else(|err| panic!("{}: {err}", target.name()));

        assert!(
            !written.is_empty(),
            "{}: a backend advertising `executable` must write at least one",
            target.name()
        );
        for path in &written {
            let bytes = std::fs::read(path)
                .unwrap_or_else(|err| panic!("{}: {}: {err}", target.name(), path.display()));
            assert!(
                bytes.len() > magic.len(),
                "{}: {} is too short to be an image",
                target.name(),
                path.display()
            );
            assert_eq!(
                &bytes[..magic.len()],
                *magic,
                "{}: {} does not open with its OS's image magic",
                target.name(),
                path.display()
            );
        }
    }
}
