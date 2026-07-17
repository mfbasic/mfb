# plan-51-C: AppImage seal

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-51-A, plan-51-B

plan-51-A produces an AppDir; plan-51-B produces squashfs bytes. This sub-plan joins
them: it embeds the upstream AppImage runtime, seals the AppDir into
`build/<name>.AppImage`, and adds the `--app-debug` flag that keeps the AppDir
around when you need to look inside it.

An AppImage is `[runtime ELF][squashfs image]` concatenated at exactly the runtime's
length. There is no container header, no alignment, and no index — the runtime
computes the boundary from its own ELF headers at startup and hands the offset to
its bundled squashfuse. So the "seal" is genuinely a concatenation plus a `chmod +x`.
The engineering is not in the joining; it is in **where the seal happens in the build
pipeline** (§3.2), because an AppImage is a sealed file and vendored libraries must
be inside it before it closes.

The behavioral outcome: `mfb build --app -target linux-x86_64` produces a single
executable `build/<name>.AppImage` that a user downloads, `chmod +x`es (already done)
and double-clicks.

References (read first):

- https://github.com/AppImage/type2-runtime — the runtime we embed. MIT.
- https://raw.githubusercontent.com/AppImage/type2-runtime/main/src/runtime/runtime.c —
  `:247-277` is the ELF-end computation §4.2 mirrors.
- https://raw.githubusercontent.com/AppImage/type2-runtime/main/LICENSE — the
  attribution §4.3 ships, including the statically-linked third-party list.
- https://raw.githubusercontent.com/AppImage/AppImageSpec/master/draft.md — `:137`,
  the magic-bytes requirement.
- `src/cli/build.rs:494-525` — `write_executable` then `copy_vendor_libraries`, the
  ordering §3.2 has to fit into.
- `src/cli/build.rs:175-179` — `--app`/`-app` parsing, the precedent `--app-debug`
  follows.
- `src/target.rs:103-169` — `NativeBackend`, where §4.5's seam goes.
- `src/target.rs:31-55` — `NativeBuildMode`.

## 1. Goal

- `mfb build --app` for `linux-x86_64`/`linux-aarch64` emits a single
  `build/<name>.AppImage`, mode 0755, and no AppDir.
- `mfb build --app-debug` emits **both** `build/<name>.AppImage` and
  `build/<name>.AppDir/`, for inspecting the payload the seal consumed.
- The AppImage runs on a stock Debian 12 / Ubuntu 22.04+ desktop by
  double-clicking, with GTK4 from the host (plan-51-A §1 non-goals) and vendored
  `libraries` from inside the image.
- The build is hermetic: no network, no `appimagetool`, no `mksquashfs`, and it
  cross-builds from macOS.
- Two builds of the same project produce byte-identical AppImages.

### Non-goals (explicit constraints)

- **No zsync / AppImageUpdate.** The runtime reserves a 1024-byte `.upd_info`
  section for a zsync URL, and it is explicitly optional ("**MAY** embed update
  information"). §4.6 records how to fill it later without disturbing the offset
  math; this plan leaves it zeroed.
- **No signing.** `.sha256_sig` and `.sig_key` stay zeroed. MFBASIC's `--sign` is
  package/artifact provenance (`src/cli/build.rs:1622`), an unrelated mechanism, and
  conflating them would be worse than leaving the section empty.
- **No desktop-integration daemon.** We emit a `.desktop` and icons that `appimaged`
  and AppImageLauncher consume if the user has them. Installing anything on the
  user's machine is not the build's job.
- **No runtime modification.** The blob ships byte-for-byte as published. We do not
  patch the magic (it is already there), do not strip it, do not relink it. Modifying
  it would make the LGPL question in §4.3 real rather than theoretical.
- **No i686 or armhf.** Upstream publishes runtimes for both, but MFBASIC has no such
  targets.
- **No riscv64** — no upstream runtime exists (plan-51-A §3.3), and app mode is
  already unsupported there.
- **No AppDir output shape change.** plan-51-A owns the tree; this sub-plan only
  reads it.

## 2. Current State

plan-51-A leaves `build/<name>.AppDir/` as the terminal Linux app-mode artifact and
plan-51-B leaves `squashfs::write` with no callers. Nothing joins them, and nothing
in the tree has ever embedded a third-party binary — the closest precedent is
`APP_ICON_PNG` (plan-51-A §4.2 relocates it to `src/os/icon.rs`), which is a
first-party asset.

The ordering that matters, `src/cli/build.rs:494-525`:

```rust
let executable_paths = target::write_executable(output_dir, &ir, &target, …)?;
// plan-46-D §4.5: copy the resolved vendor libraries into the directory
// the executable's RPATH points at …
if let Err(err) = copy_vendor_libraries(
    &vendored, &options.location,
    &vendor_output_dirs(output_dir, &ir.name, build_mode),
) { … }
…
for executable_path in executable_paths {
    println!("Wrote executable to {}", executable_path.display());
}
```

`write_executable` returns paths, and **vendoring happens after it returns**. For
macOS that is harmless: a `.app` is a directory, so copying dylibs into
`Contents/Frameworks/` after the fact is fine. An AppImage is a sealed file. §3.2 is
about that difference.

`--app` parsing (`src/cli/build.rs:175-179`) accepts `--app` and the legacy `-app`,
errors on a duplicate, and lands in `BuildOptions.app_mode` (`:87`, `:212`).
`build_mode` is then chosen by target OS at `:260-267`.

## 3. Design Overview

Four pieces:

1. **The embedded runtime** (§4.1–4.3) — two checked-in blobs plus the upstream
   LICENSE, selected by target arch.
2. **The seal** (§4.4) — read the AppDir into a `SquashTree`, serialize, concatenate,
   `chmod +x`.
3. **A pipeline seam** (§4.5) — a `finalize_app_bundle` hook that runs *after*
   vendoring, which is the only correct place for it.
4. **`--app-debug`** (§4.7) — retention of the intermediate AppDir.

The correctness risk is **not** in the concatenation, which is trivially verifiable.
It is in **§4.2's offset assumption** — that the squashfs starts at exactly
`runtime.len()` — which is true for the published blobs but is a property of *those
blobs*, not of the format. §4.2 turns it into an asserted invariant rather than a
belief.

### 3.1 Why embed the runtime rather than download it

Downloading at build time would keep ~1.86 MB out of the repo. It would also mean the
compiler cannot build offline, cannot build reproducibly (upstream retags
`continuous`), and acquires a network dependency in service of emitting a file. This
tree has a built-in linker specifically so that `mfb build` depends on nothing but
`mfb` — an AppImage that needs GitHub to be up would be the only artifact that does.

Writing our own runtime was considered and is not close: it is a squashfuse plus
libfuse reimplementation, weeks of work, to replace a 920 KiB MIT blob that upstream
maintains and that every AppImage on earth already runs.

The cost is real and worth stating: **the `mfb` binary grows by ~1.86 MB**, on every
platform, including macOS hosts that will never emit a Linux AppImage. `#[cfg]`-ing
it out is not available — cross-compiling from macOS to Linux is the primary
workflow, so the macOS build is exactly the one that needs the Linux blobs.

### 3.2 Why the seal runs after vendoring, and what that costs

`copy_vendor_libraries` runs after `write_executable` returns and writes into the
directory `vendor_output_dirs` names (`src/cli/build.rs:1484-1506`). plan-51-A §4.4
points that at `build/<name>.AppDir/usr/lib/`. So at the moment `write_executable`
finishes, **the AppDir is incomplete** — the ELF is there but its libraries are not.

Three options:

**(a) Seal in a post-vendor step.** `write_executable` emits the AppDir;
`copy_vendor_libraries` fills `usr/lib/`; a new `finalize_app_bundle` step seals.
Chosen. It preserves `vendor_output_dirs` as the single RPATH↔directory table —
the thing its doc comment insists must stay in lockstep with the backends — and it
keeps the "vendor libraries go where the RPATH points" invariant literally true, with
`usr/lib/` a real directory at the moment of copying.

**(b) Move vendoring into the AppDir writer.** Rejected. It would give
`write_executable` a second job, duplicate the hash-verified copy logic that
`copy_vendor_libraries` owns, and split the RPATH table across two places. plan-46-D
§4.5 is explicit that the copy must consume the same verified bytes without
re-hashing; reimplementing that inside a backend invites exactly the drift the table
exists to prevent.

**(c) Build the squashfs from an in-memory tree, never touching disk.** Genuinely
attractive — no intermediate, no cleanup, no `--app-debug` question. Rejected because
vendored libraries are real files on disk that `copy_vendor_libraries` writes, so an
in-memory tree would have to read them back anyway, and because it deletes the
AppDir as a debuggable artifact (plan-51-A §3.1 argues that is the whole point of
having one).

The cost of (a): the AppDir is written to disk and then, in the default `--app` case,
deleted. That is one extra tree write per build. It is also what makes `--app-debug`
a one-line retention flag rather than a second code path.

### 3.3 Why the AppDir is deleted by default

`--app` emits one artifact, matching macOS `--app`'s single `.app`. Leaving an AppDir
beside every AppImage would mean the same payload twice, and the directory is the one
nothing points at. `--app-debug` keeps it for the case where you need to see what
went in.

## 4. Detailed Design

### 4.1 The AppImage file

```text
offset 0            the runtime ELF, byte-for-byte as published
offset runtime.len() the squashfs image (plan-51-B)
```

No header, no padding, no alignment. The runtime finds the boundary itself
(`runtime.c:247-277`):

```c
/* ELF ends either with the table of section headers (SHT) or with a section. */
sht_end = ehdr.e_shoff + (ehdr.e_shentsize * ehdr.e_shnum);
last_section_end = file64_to_cpu(shdr64.sh_offset) + file64_to_cpu(shdr64.sh_size);
return sht_end > last_section_end ? sht_end : last_section_end;
```

and passes `ro,offset=%zu` to squashfuse.

**Alignment: none, and padding is actively wrong.** The published `runtime-x86_64` is
944632 bytes — `% 4096 == 2552`, `% 512 == 504`, deliberately unaligned. Any padding
between the runtime and the squashfs would be read as the superblock and fail the
mount. [AppImageKit PR #602](https://github.com/AppImage/AppImageKit/pull/602),
which proposed 512-byte padding, remains open and unmerged.

**The magic.** The spec requires hex `0x414902` at offset 8 — bytes `AI\x02`, sitting
in `EI_ABIVERSION` and `EI_PAD`, which the Linux kernel ignores:

```
00000000: 7f45 4c46 0201 0100 4149 0200 0000 0000  .ELF....AI......
```

**The published blob already has it** (upstream's `build-runtime.sh` `dd`s it in
post-strip) and **the runtime never reads it** — `grep` for it in `runtime.c` returns
nothing. It exists purely for external tools (`file`, AppImageLauncher, desktop
integration). We patch nothing; we assert it is present (§4.2).

### 4.2 The offset invariant

The squashfs goes at `runtime.len()`. That is correct **iff** the runtime's
ELF-derived end equals its file length — i.e. iff the section header table is the
last thing in the file. For the published blobs it is (verified: the computation
yields exactly 944632 for `runtime-x86_64`, with `sht_end` 944632 beating
`last_section_end` 942712). But that is a property of how upstream links *these*
blobs, not a guarantee of the format. A future blob with trailing data would silently
place our squashfs at the wrong offset, and the failure would be a mount error.

So: **implement the same computation and assert it equals `len()`.**

```rust
/// The offset the AppImage runtime will look for the squashfs at (plan-51-C §4.2):
/// the end of its own ELF, computed exactly as `runtime.c:247-277` does.
///
/// For every runtime upstream has published this equals the blob's length, so the
/// seal appends at `blob.len()`. That is a property of how upstream links the
/// blob, not of the format — a future blob with trailing data would put our
/// squashfs somewhere the runtime does not look, and the only symptom would be a
/// mount failure on a user's desktop. This is the assertion that turns a blob
/// swap into a failing unit test.
fn elf_image_end(runtime: &[u8]) -> Result<u64, String>
```

A unit test asserts `elf_image_end(blob)? == blob.len()` for each embedded runtime,
and that bytes 8–10 are `AI\x02`. Both run on every `cargo test`, so replacing a blob
without re-verifying it is not possible.

### 4.3 The embedded blobs

```text
src/os/linux/appimage/
  mod.rs
  runtime-x86_64          944,632 bytes
  runtime-aarch64         936,456 bytes
  LICENSE.type2-runtime
```

Pinned to release tag **20251108**, from
`https://github.com/AppImage/type2-runtime/releases/download/<tag>/runtime-<arch>`.
Not `continuous` — it is a rolling tag and reproducibility requires a fixed one.
Upstream publishes GPG `.sig` files and a `signing-pubkey.asc`; the acquisition is a
one-time manual step and §5 Phase 1 records verifying the signature before committing.

```rust
const RUNTIME_X86_64: &[u8] = include_bytes!("runtime-x86_64");
const RUNTIME_AARCH64: &[u8] = include_bytes!("runtime-aarch64");

/// The AppImage type-2 runtime for `arch` (plan-51-C §4.3).
///
/// Embedded rather than downloaded so `mfb build` stays hermetic and offline —
/// the same reason this compiler has a built-in linker. Cross-building Linux from
/// macOS is the primary workflow, so these cannot be `#[cfg]`-gated to Linux
/// hosts: the macOS build is precisely the one that needs them.
fn runtime_for(arch: &str) -> Result<&'static [u8], String>
```

Each is a static-PIE musl binary bundling **libfuse 3.15.0**, squashfuse, zstd, and
zlib. **libfuse2 is not required** on the host — a common misconception from older
AppImages. What *is* required is a setuid `fusermount`/`fusermount3` on the host's
`$PATH` plus `/dev/fuse` (§4.6).

**Licensing.** The runtime is **MIT**, `Copyright (c) 2004-23 probonopd`. We ship its
LICENSE verbatim as `LICENSE.type2-runtime` and reference it from the repo's
top-level license notice. The upstream LICENSE additionally enumerates the
statically-linked third-party code: musl libc, **libfuse (LGPL-2.1)**, squashfuse
(BSD-2), libzstd, and zlib.

⚠️ **Flagged for the maintainer, not decided here:** LGPL-2.1 static linking normally
carries a relinking obligation. We redistribute an *unmodified* blob that the
AppImage project itself publishes under these terms, which is the ordinary and
intended use, and §1 non-goals forbid modifying it — but this is a licensing call,
not an engineering one, and it should be looked at before this sub-plan ships rather
than after. Open Decisions records it.

### 4.4 The seal

```rust
/// Seal an AppDir into a single-file AppImage (plan-51-C §4.1):
/// `[runtime ELF][squashfs]`, concatenated at the runtime's exact length.
///
/// The output is `chmod +x`: an AppImage without the executable bit is a file the
/// user cannot run and gets no diagnostic from beyond "Permission denied".
///
/// Deterministic — the runtime is a fixed blob and the squashfs sets
/// `mkfs_time`/`mtime` to 0 (plan-51-B §4.2), so two builds of one project are
/// byte-identical.
pub(crate) fn seal(
    project_dir: &Path,
    project_name: &str,
    arch: &str,
) -> Result<PathBuf, String>
```

Steps:

1. `read_appdir(build/<name>.AppDir) -> SquashTree` — walk the tree into plan-51-B's
   input type. Preserves mode bits (the ELF's 0755 must survive; verified to
   round-trip as `-rwxr-xr-x`) and reads symlink targets with
   `fs::read_link`, **not** following them: `AppRun` and `.DirIcon` must stay
   symlinks in the image.
2. `squashfs::write(&tree)` → bytes.
3. `runtime_for(arch)` → blob; assert §4.2.
4. Concatenate into `build/<name>.AppImage`; `set_mode(0o755)`.

`read_appdir` is the one place a symlink must not be followed. `fs::metadata` follows;
`fs::symlink_metadata` does not. Getting this wrong turns `AppRun` into a second copy
of the ELF — which still *works*, silently, at double the size.

### 4.5 The pipeline seam

```rust
// src/target.rs, beside supports_app_mode
/// Finalize an app-mode build after vendored libraries are in place
/// (plan-51-C §3.2).
///
/// Runs after `copy_vendor_libraries`, which is the only correct point: a sealed
/// artifact cannot gain files afterwards. Returns the path that replaces what
/// `write_executable` reported, or `None` to keep it.
///
/// macOS returns `None` — a `.app` is a directory and is already complete.
fn finalize_app_bundle(
    &self,
    project_dir: &Path,
    project_name: &str,
    keep_intermediate: bool,
) -> Result<Option<PathBuf>, String> {
    let _ = (project_dir, project_name, keep_intermediate);
    Ok(None)
}
```

`linux_x86_64`/`linux_aarch64` override it: seal, then unless `keep_intermediate`,
`remove_dir_all` the AppDir, and return the AppImage path.

The CLI (`src/cli/build.rs:494-525`) gains one step between vendoring and printing:

```rust
let executable_paths = target::write_executable(…)?;          // -> build/<name>.AppDir
copy_vendor_libraries(&vendored, …, &vendor_output_dirs(…))?; // -> …/usr/lib/
// plan-51-C §3.2: seal the AppDir into build/<name>.AppImage. Must run after
// vendoring — the libraries have to be inside the image before it closes.
let executable_paths = target::finalize_app_bundle(
    output_dir, &ir.name, &target, build_mode, options.app_debug,
)?.map_or(executable_paths, |path| vec![path]);
```

`mfb test` already rejects `--app` (`src/cli/build.rs:1800-1801`), so the
`run_test_binary` path at `:533` never sees an AppImage and needs no change.

### 4.6 What the runtime does at launch

Worth recording, because each of these is a support question waiting to happen:

- **Mounts via FUSE and `execv`s `<mount>/AppRun`.** It needs a setuid
  `fusermount`/`fusermount3` on `$PATH` and `/dev/fuse`. Present on stock Debian 12
  and Ubuntu 22.04+.
- **There is no automatic fallback.** If the mount fails it prints *"Cannot mount
  AppImage, please check your FUSE setup"* and exits. It does **not** self-extract.
  The caller opts in with `--appimage-extract-and-run` or `APPIMAGE_EXTRACT_AND_RUN=1`.
- **Sets `APPIMAGE`, `ARGV0`, `APPDIR`**, and `OWD` (original working directory) —
  but `OWD` **only on the mount path, not under extract-and-run**. A real behavioral
  difference, observed during research.
- **Honors** `TARGET_APPIMAGE`, `NO_CLEANUP`, `FUSERMOUNT_PROG`, `TMPDIR`.
- **Reserves patchable placeholder sections**, zero-filled, at fixed offsets:
  `.digest_md5` (16 B), `.upd_info` (1024 B), `.sha256_sig` (1024 B), `.sig_key`
  (8192 B). They are patched **in place**, so the ELF size — and therefore §4.2's
  offset — never moves. This is the mechanism a future zsync/signing plan would use;
  it must never add or resize a section, which would shift `e_shoff` and break
  everything.

⚠️ **The AI magic breaks qemu-user, and this shapes the whole test story.**
`EI_ABIVERSION = 0x41` is ignored by the real kernel but **rejected by
qemu-user/Rosetta's ELF loader**. Proven during research: with the magic → `applet
not found` (exec fails); with bytes 8–10 zeroed → runs fine. Upstream notes the same
in PR #602. **An x86_64 AppImage cannot be smoke-tested under Docker on Apple
Silicon.** Testing requires a real kernel — a full-system VM is fine, binfmt
emulation is not. plan-51-D owns this.

### 4.7 `--app-debug`

Parsed beside `--app` (`src/cli/build.rs:175-179`), same duplicate check, landing in
`BuildOptions.app_debug`.

**`--app-debug` implies `--app`.** `mfb build --app-debug` is app mode with the AppDir
kept; `--app --app-debug` is the same thing said twice and is accepted. Requiring both
would be a papercut with no upside.

It is Linux-only in effect, not in acceptance: on macOS `finalize_app_bundle` returns
`None` and the flag does nothing, because there is no intermediate to keep. Erroring
on `--app-debug -target macos-aarch64` would mean a flag that changes a build's
*validity* by target, which is worse than one that changes nothing. This is worth a
line in the CLI reference rather than a diagnostic.

`--app-debug` alone must not resurrect `<name>.out`: the AppDir *is* the debug
artifact, and it is directly runnable (plan-51-A §3.1).

## Compatibility / Format Impact

**Changes:**

- `mfb build --app` on `linux-x86_64`/`linux-aarch64` emits `build/<name>.AppImage`
  (0755) instead of plan-51-A's `build/<name>.AppDir/`. The `Wrote executable to …`
  line prints the AppImage.
- New CLI flag `--app-debug`, accepted by `mfb build` only, rejected by `mfb test`
  alongside `--app`.
- The `mfb` binary grows ~1.86 MB on every host (§3.1).
- New third-party attribution obligation: `LICENSE.type2-runtime` (§4.3).

**Unchanged:**

- The AppDir layout, the `.desktop`, the icons, the RUNPATH, the GTK app id — all
  plan-51-A's, all consumed as-is.
- Every console artifact and every macOS artifact.
- `mfb build --app` remains rejected for riscv64 and non-executable projects.
- The `.mfp` format, NIR/nplan `"buildMode": "linux-app"`, the manifest schema.

## Phases

### Phase 1 — Embed and assert the runtime

No behavior change; lands alone and makes every later phase's offset math trustworthy.

- [ ] Download `runtime-x86_64` and `runtime-aarch64` from release tag `20251108`;
      **verify the upstream GPG signature against `signing-pubkey.asc` before
      committing**. Record the tag and the SHA-256 of each blob in `mod.rs`.
- [ ] Add `src/os/linux/appimage/mod.rs` with the blobs, `LICENSE.type2-runtime`,
      `runtime_for(arch)`, and `elf_image_end` (§4.2).
- [ ] Reference `LICENSE.type2-runtime` from the repo's top-level license notice.
- [ ] Tests: for each blob — `elf_image_end(blob)? == blob.len()`; bytes 8–10 are
      `AI\x02`; bytes 0–3 are `\x7fELF`; the length matches the recorded constant.

Acceptance: `cargo test` green; swapping either blob for a stale or truncated copy
fails a test rather than shipping.
Commit: —

### Phase 2 — Read an AppDir into a SquashTree

The half of the seal that can be tested without a runtime.

- [ ] Add `read_appdir(path) -> SquashTree` per §4.4, using `symlink_metadata` so
      symlinks are read as symlinks.
- [ ] Tests: round-trip a fixture AppDir — `AppRun` and `.DirIcon` are `Symlink`
      nodes with the right targets and are **not** duplicated files; `usr/bin/<name>`
      is a `File` with mode 0755; a `usr/lib/` present only when vendoring.

Acceptance: `read_appdir` on a plan-51-A AppDir yields a tree whose symlinks are
symlinks; `squashfs::write` of it succeeds and `unsquashfs` extracts a tree matching
the original, including modes and link targets.
Commit: —

### Phase 3 — The seal and the pipeline seam

- [ ] Add `seal` per §4.4.
- [ ] Add `NativeBackend::finalize_app_bundle` (§4.5) defaulting to `Ok(None)`;
      override in `linux_x86_64`/`linux_aarch64`.
- [ ] Wire the CLI step between `copy_vendor_libraries` and the `Wrote executable to`
      print (`src/cli/build.rs:494-525`).
- [ ] Tests: the sealed file's first `runtime.len()` bytes equal the blob exactly; a
      valid squashfs superblock (`hsqs`) begins at exactly that offset; the file is
      0755; two builds are byte-identical; the AppDir is gone after a plain `--app`.

Acceptance: `mfb build --app -target linux-x86_64` on a fixture emits
`build/<name>.AppImage` and no AppDir; `unsquashfs -o <runtime.len()> -l` lists the
full §4.1 tree.
Commit: —

### Phase 4 — `--app-debug`

- [ ] Parse `--app-debug` (`src/cli/build.rs:175-179`), add `BuildOptions.app_debug`,
      make it imply `app_mode` (`:239`).
- [ ] Reject it in `mfb test` beside `--app` (`:1800-1801`).
- [ ] Tests: `--app-debug` parses and implies app mode; duplicate errors;
      `mfb test --app-debug` is rejected; a `--app-debug` build leaves both the
      AppImage and the AppDir; a plain `--app` build leaves only the AppImage; on
      `macos-aarch64` the flag is accepted and changes nothing.

Acceptance: both artifacts present after `--app-debug`, one after `--app`, and the
AppImage bytes are identical between the two.
Commit: —

### Phase 5 — Runtime proof (highest-risk work last)

Everything above is verifiable from macOS. This is not, and it is the only phase that
proves the artifact does what it exists to do.

- [ ] On the Ubuntu x86_64 GTK box (`ssh -p 2228`) and the Debian 12 GTK box
      (`ssh -p 2226`): `./<name>.AppImage` opens a GTK window titled `<name>`.
- [ ] Verify `--appimage-extract-and-run` also works (the no-FUSE path).
- [ ] Verify a vendoring fixture loads its library from inside the image with no
      `LD_LIBRARY_PATH`.
- [ ] Verify `./<name>.AppImage --appimage-extract` yields a tree matching a
      `--app-debug` AppDir from the same build.

Acceptance: a window opens on both boxes from a double-clickable single file, and the
extracted tree matches the AppDir byte-for-byte.
Commit: —

## Validation Plan

- **Tests:** unit — blob assertions (§4.2), `read_appdir` symlink handling,
  concatenation offset, determinism, mode bits. Integration — extend
  `tests/linux_app_mode.rs` (cross-builds and inspects without executing; the dev
  host is macOS) to assert the AppImage's magic, the superblock at `runtime.len()`,
  and the artifact set per flag. Negative — duplicate `--app-debug`, `mfb test
  --app-debug`, `--app` on riscv64.
- **Runtime proof:** Phase 5. ⚠️ **This cannot be done under Docker/QEMU on the Mac**
  — §4.6's magic-vs-qemu-user finding means an emulated loader refuses the file
  before the runtime runs. It needs a real kernel: the GTK boxes (2226 Debian 12,
  2228 Ubuntu x86_64) are the targets, and they are also the only ones with GTK4 and
  FUSE. The musl boxes are irrelevant (app mode is glibc-only) and the riscv64 box
  has no runtime.
- **Doc sync:** `src/docs/spec/architecture/08_artifacts.md:46-49` — add the
  `build/<name>.AppImage` row (note the table lists no Linux `--app` output at all
  today). `src/docs/spec/tooling/07_cli-reference.md:130,147-151` — the `--app` flag
  table gains `--app-debug`; the output-shape prose changes.
  `src/docs/spec/app/02_linux-runtime.md` — the app-mode artifact is now an AppImage.
  plan-51-D owns the full sweep.
- **Acceptance:** `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo fmt`
  (second pass in `repository/`). No golden churn expected — no `linux-*.app.*`
  goldens exist.

## Open Decisions

- ⚠️ **The libfuse LGPL-2.1 static-linking question (§4.3)** — recommend proceeding:
  we redistribute an unmodified blob that upstream publishes under exactly these
  terms, which is the intended use, and §1 forbids modifying it. But this is a
  licensing call and wants a maintainer's eye **before** shipping, not after. It is
  the only decision here that is not reversible by editing code.
- **Pin tag `20251108` vs. `continuous`** — recommend the pinned tag. `continuous`
  rolls, which would make AppImages non-reproducible and turn an upstream push into a
  silent change in our output. Bumping the pin is a deliberate commit with a
  re-verified signature.
- **Delete the AppDir by default (§3.3)** — recommend yes, matching macOS's single
  `.app`. `--app-debug` covers the inspection case.
- **`.upd_info` / zsync (§1, §4.6)** — recommend leaving zeroed. The placeholder
  sections make it a clean follow-up whenever there is a distribution channel to
  point at; there is not one today.

## Summary

The concatenation is the easy part, and it is not where this can go wrong.

The real risk is **§4.2's offset invariant**: everything works because the published
runtimes happen to end at their section header table, so `runtime.len()` is the right
append point. That is upstream's linking decision, not a format guarantee, and a
future blob that breaks it would produce an AppImage that fails to mount with no
build-time signal. The mitigation is to implement upstream's own end-of-image
computation and assert it matches — cheap, and it converts a blob swap from a field
failure into a failing unit test.

The second risk is **structural, not technical**: §3.2's ordering. A sealed artifact
cannot gain files after it closes, so the seal must run after `copy_vendor_libraries`.
Getting that wrong yields an AppImage that builds cleanly, runs on the developer's
box, and fails only for projects that vendor a library — the narrowest, latest-caught
failure available.

The third is **§4.6's qemu-user finding**, which is a testing risk rather than a
product one: the AI magic that makes the file an AppImage also makes it unrunnable
under emulation, so the only proof that matters needs real hardware. Phase 5 is not
optional and cannot be shortcut from the Mac.

Left untouched: the AppDir and everything in it (plan-51-A), the squashfs writer
(plan-51-B), every console and macOS artifact, and the runtime blob itself — which we
ship exactly as published, deliberately.
