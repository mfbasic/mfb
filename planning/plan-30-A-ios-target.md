# plan-30-A: iOS Simulator build target (foundation)

Last updated: 2026-07-07
Overall Effort: huge (>3d)   <!-- the whole plan-30 feature: iOS + hand-rolled Swift async ABI + StoreKit 2 -->
Effort: medium (1h–2h)

This sub-plan adds a new **iOS Simulator aarch64** build target: the compiler
emits an arm64 Mach-O whose load commands identify it as an iOS-Simulator binary
(`LC_BUILD_VERSION` platform `PLATFORM_IOSSIMULATOR`), links against the
iPhoneSimulator SDK, is ad-hoc code-signed, and boots on a running simulator to
produce observable output. It is the prerequisite every other plan-30 sub-plan
sits on: you cannot run *any* iOS code — UIKit or StoreKit — without a Mach-O the
simulator will load.

The single behavioral outcome: `mfb build` for the new target produces a Mach-O
that `xcrun simctl` runs on a booted iOS simulator and whose load commands match,
byte-for-structure, an oracle binary the real Apple toolchain produces for the
same target. **No UI and no Swift/StoreKit in this sub-plan** — console/no-UI
process only.

It complements:

- `mfb spec linker macos-aarch64` (`src/docs/spec/linker/06_macos-aarch64.md` — the Mach-O emission this parameterizes)
- `mfb spec app spec` (`src/docs/spec/app/**` — the app-mode runtime that 30-C will grow a UIKit backend for)

## plan-30 feature map (the whole `NN`)

Split by effort; each letter is an independently-landable small/medium plan.

- **30-A — iOS Simulator target (this doc).** Mach-O platform bytes + SDK link line + ad-hoc sign + simulator boot proof. No UI, no Swift.
- **30-B — iOS `.app` bundle + install.** iOS bundle layout (flat, no `Contents/`), iOS `Info.plist` keys, `xcrun simctl install`/`launch` path. Depends on A.
- **30-C — UIKit app runtime backend.** `UIApplicationMain` + a runtime-registered app-delegate class (`objc_allocateClassPair`/`class_addMethod`, hand-emitted), `UIWindow`/`UIViewController`/`UITextView`, worker pthread — sibling to `src/target/macos_aarch64/app/`. Reuses `performSelectorOnMainThread:` for worker→UI. Depends on A, B.
- **30-D — Swift async ABI bridge (the hard research spike).** Hand-emit the swiftcc async calling convention for a *fixed* call set: async-frame alloc (`swift_task_alloc`/`swift_task_dealloc`), continuation + executor hop (`swift_task_switch`), a top-level task to drive it, plus swiftcc value/metadata/ARC plumbing. Every sequence reverse-engineered from and byte-validated against the `swiftc` **development-time oracle** (the shipped `mfb` invokes no external compiler). Depends on A (needs a runnable iOS process to prove against). This is where the correctness risk concentrates.
- **30-E — `iap::` package over StoreKit 2.** `iap::products`/`iap::purchase`/`iap::observe` calling `Product.products(for:)`, `Product.purchase(options:)`, `Transaction.updates`/`finish()` through the D bridge, bridged to the worker thread over the existing inbound/outbound resource channel (`THREAD_OFFSET_RESOURCE_INBOUND_QUEUE`=104 / `OUTBOUND`=112, `src/target/shared/code/runtime_helpers.rs:26`). Verified transactions come from SK2's `VerificationResult.verified` (JWS) — no separate receipt-server. Depends on C, D.

Evidence the D/E surface is bounded (measured against iPhoneSimulator SDK 26.2,
Swift 6.2): the entire concurrency-runtime surface for fetch-products + purchase +
observe is **three** entry points (`swift_task_alloc`, `swift_task_dealloc`,
`swift_task_switch`); ~45 external symbols total; the async StoreKit functions are
`…Tu` async-function-pointer symbols referenced by name, not code we synthesize.

## 1. Goal

- A new `BuildTarget` (working name `ios-sim-aarch64`) selectable from the CLI that
  emits an arm64 Mach-O executable with `LC_BUILD_VERSION` platform
  `PLATFORM_IOSSIMULATOR` (7), a simulator-appropriate `minos`/`sdk`, and
  `LC_LOAD_DYLIB` install-names resolvable inside the simulator runtime.
- The emitted binary is ad-hoc code-signed (`codesign -v` passes) and **runs on a
  booted iOS simulator** via `xcrun simctl`, producing observable output.
- The binary's load-command structure matches an **oracle** binary produced by the
  Apple toolchain (`clang`/`swiftc`) for the same target, verified by `otool -l`
  diff.

### Non-goals (explicit constraints)

- **No change to any existing target's output bytes.** The macOS aarch64 and Linux
  targets must remain byte-identical; platform/minos become target-derived
  parameters, not new constants, and the macOS path passes the same values it does
  today. This is a hard regression guard.
- No language-surface, value/copy/move/freeze, layout, or thread-transfer change —
  this sub-plan is entirely in the object-file/linker layer.
- No UIKit, no `.app` bundle (30-B), no Swift/StoreKit (30-D/E). Console process only.
- No external compiler or linker in the shipped `mfb` build path; `swiftc`/`clang`
  are development-time oracles for producing reference binaries only.
- No real-device target and no provisioning/entitlements — simulator only.

## 2. Current State

- Targets: `BuildTarget` at `src/target.rs:16`; backends registered at
  `src/target.rs:155` (`macos_aarch64`, `linux_aarch64`, x86_64 Linux). No iOS
  target, no simulator notion.
- Mach-O platform is a **constant**: `build_version()` at
  `src/os/macos/link/commands.rs:315` emits `LC_BUILD_VERSION` (cmd `0x32`, size 32)
  with `platform = 1` (`PLATFORM_MACOS`), `minos = 11 << 16` (11.0), `sdk = 0`,
  `ntools = 1`. The load-command-plan mirror is `src/os/macos/object.rs:317`.
- Mach-O header: `src/os/macos/link/macho.rs:63` — magic `0xfeedfacf`, cputype
  `0x0100000c` (`CPU_TYPE_ARM64`), filetype `2` (`MH_EXECUTE`). arm64 already the
  only macOS arch; the iOS-simulator arch is *also* arm64, so the header is reusable.
- Dylib install-name whitelist: `dylib_for_library()` at `src/os/macos/object.rs:616`
  (mirror `src/os/macos/link/mod.rs:294`) — a fixed name→path table; anything absent
  is a hard error.
- Ad-hoc code signing already exists: `LC_CODE_SIGNATURE` / `mfb_sign_segment` in
  `src/os/macos/link/macho.rs` (~line 100) and `src/os/macos/object.rs`.
- Precedent to mirror: the target is a thin variant of `macos_aarch64` — same
  instruction selection, same Mach-O writer, differing only in the build-version
  bytes and the SDK/dylib resolution. Treat it as a parameterization of the existing
  macOS backend, not a new backend.

## 3. Design Overview

Three independent, ordered pieces, each landable alone:

1. **Parameterize the Mach-O platform** (Phase 1). Replace the two hardcoded
   `PLATFORM_MACOS` constants with values carried by the target descriptor. macOS
   keeps passing `(platform=1, minos=11.0)`; the new target passes
   `(platform=7, minos=<ios>.0)`. Guarded by a byte-identity check on the macOS
   golden path.
2. **Resolve the simulator SDK & dylib paths** (Phase 2). The simulator links the
   same logical dylibs (`libSystem`, `Foundation`, later `UIKit`/`StoreKit`) but the
   install-names and the SDK search root differ. Derive the correct `LC_LOAD_DYLIB`
   strings and SDK path from the target, verified against an oracle binary's
   `otool -l`.
3. **Boot proof** (Phase 3). Emit a minimal console process, ad-hoc sign it, and run
   it on a booted simulator via `xcrun simctl`, capturing observable output.

Correctness risk concentrates in Phase 2 (getting the load commands exactly right so
`dyld` inside the simulator will map the image) — which is why the plan pins every
byte against an Apple-toolchain oracle rather than reasoning from docs.

## 4. Detailed Design

### 4.1 Target descriptor & platform parameters

Add an `ios-sim-aarch64` variant to `BuildTarget` (`src/target.rs:16`) and register
its backend (`src/target.rs:155`). Because instruction selection is identical to
`macos_aarch64`, the backend delegates to the macOS backend for everything except:

- a `mach_platform()` accessor → `1` (macOS) vs `7` (iOS simulator);
- a `min_os_version()` accessor → the target's minimum OS (see Open Decisions);
- an `sdk_root()` accessor → `xcrun --sdk iphonesimulator --show-sdk-path` result,
  threaded to dylib resolution.

`build_version()` (`src/os/macos/link/commands.rs:315`) and its plan mirror
(`src/os/macos/object.rs:317`) take these as parameters instead of literals. The
macOS caller supplies `(1, 11<<16, 0)` — its current values — so its bytes do not
move.

### 4.2 Simulator dylib resolution

Extend `dylib_for_library()` (`src/os/macos/object.rs:616` + mirror
`src/os/macos/link/mod.rs:294`) to select install-names per target. Method: build an
oracle (`clang -target arm64-apple-ios18.0-simulator -isysroot <sdk> hello.c`),
`otool -l` it, and copy the exact `LC_LOAD_DYLIB` `name` strings and the
`LC_BUILD_VERSION`/`LC_MIN_VERSION` bytes. Encode those as the simulator table.

### 4.3 Boot & observe

The simulator loads Mach-O images that are (a) correctly platform-stamped and (b)
code-signed. Ad-hoc signing (existing `mfb_sign_segment`) suffices. Run via
`xcrun simctl spawn booted <path>` for a bare executable (no bundle needed until
30-B), capturing stdout/stderr, or `os_log` observed through `xcrun simctl spawn
booted log stream`.

## Layout / ABI Impact

Only the **new** target's Mach-O `LC_BUILD_VERSION` (and its simulator
`LC_LOAD_DYLIB` names) differ from the macOS target. The macOS and Linux targets
emit identical bytes to today — enforced by a golden byte-identity check in Phase 1.
No `mfb spec memory` / `mfb spec package` change; no language-visible layout,
copy/transfer, or golden-output change.

## Phases

### Phase 1 — Target registration & platform parameterization

Adds the target and turns the two hardcoded platform constants into target-derived
parameters, with the macOS path proven byte-identical.

- [ ] Add `ios-sim-aarch64` to `BuildTarget` and register the delegating backend (`src/target.rs:16`, `src/target.rs:155`).
- [ ] Thread `mach_platform()` / `min_os_version()` into `build_version()` and its plan mirror (`src/os/macos/link/commands.rs:315`, `src/os/macos/object.rs:317`); macOS passes its current `(1, 11<<16)`.
- [ ] Regression guard: assert the macOS target's emitted Mach-O is byte-identical to a pre-change golden for a representative program (artifact-gate path, `scripts/artifact-gate.sh`).
- [ ] Tests: target-selection unit coverage (the new target is listed/selectable; unknown-target still errors).

Acceptance: macOS/Linux artifacts are byte-identical to before (golden diff empty),
and building the new target emits a Mach-O whose `LC_BUILD_VERSION` shows
platform 7 with the chosen minos (`otool -l`).
Commit: —

### Phase 2 — Simulator SDK link line & dylib install-names

Makes the emitted image linkable/loadable inside the simulator, pinned to an oracle.

- [ ] Produce an oracle binary (`clang -target arm64-apple-ios18.0-simulator -isysroot $(xcrun --sdk iphonesimulator --show-sdk-path)`) and record its `LC_LOAD_DYLIB` names + build-version bytes.
- [ ] Add the simulator install-name table and SDK-root resolution to `dylib_for_library()` and its mirror (`src/os/macos/object.rs:616`, `src/os/macos/link/mod.rs:294`).
- [ ] Ad-hoc sign the simulator image via the existing `mfb_sign_segment` path; confirm `codesign -v` passes.

Acceptance: `otool -l` of the mfb-emitted binary matches the oracle's load-command
structure (platform, minos, dylib names), and `codesign -v <binary>` succeeds.
Commit: —

### Phase 3 — Boot proof on a running simulator (highest-risk: real dyld load)

- [ ] Emit a minimal console program (writes a known line, exits 0) for the new target.
- [ ] Boot a simulator and run it (`xcrun simctl boot`, `xcrun simctl spawn booted <binary>`), capturing output; document the exact reproducible command sequence in the plan/spec.

Acceptance: the mfb-built binary runs on a booted iOS simulator and emits the
expected line (captured via `simctl spawn` stdout or `log stream`); the command
sequence is recorded and reproducible.
Commit: —

## Validation Plan

- Function tests: none — this sub-plan adds no `mfb`-language surface. Coverage is
  target-selection unit tests + the byte-identity regression guard (Phase 1).
- Runtime proof: Phase 3 — a real binary running on a booted simulator emitting a
  known line (execution proof, not golden output).
- Doc sync: add a simulator section to `src/docs/spec/linker/**` documenting the
  `PLATFORM_IOSSIMULATOR` build-version bytes and the simulator dylib install-names;
  note the new target in the linker spec overview.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` passes
  unchanged (existing targets unaffected).

## Open Decisions

- **Minimum iOS version** — recommend **17.0** (broad device reach, well within the
  26.2 SDK) vs. matching the SDK's latest. (§4.1)
- **Boot vehicle for Phase 3** — recommend **bare executable via `simctl spawn`**
  (no bundle dependency, keeps A independent of B) vs. deferring the boot proof to
  30-B's `.app` install. (§4.3)
- **Target name** — recommend `ios-sim-aarch64` (explicit about simulator) vs.
  `ios-aarch64` (reserve the unqualified name for a future device target). (§1)

## Non-Goals

- Real-device target, provisioning profiles, entitlements, `LC_ENCRYPTION_INFO`,
  App Store packaging — a later plan if/when device deployment is wanted.
- Any UIKit, `.app` bundle, or Swift/StoreKit code (30-B/C/D/E).

## Summary

The real engineering risk is Phase 2/3: getting `LC_BUILD_VERSION` and the
`LC_LOAD_DYLIB` install-names exactly right so the simulator's `dyld` maps the
image — de-risked by pinning every byte to an Apple-toolchain oracle instead of
reasoning from documentation. Everything else is a thin parameterization of the
existing, working `macos_aarch64` Mach-O path, and the macOS/Linux targets are held
byte-identical by an explicit regression guard. Nothing in the language, runtime
layout, or thread-transfer model is touched.
