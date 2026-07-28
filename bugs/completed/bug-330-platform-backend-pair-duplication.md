# bug-330: the macOS/Linux backend pairs in `src/target/shared/code/` implement the same surface twice, and `tls` has become the de-facto home for generic native-codegen helpers

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: Fixed (2026-07-23)
Regression Test: none new in the failing-then-passing sense (behavior-preserving);
substitute is a per-target `.ncode` byte-identity harness + new EC `.ncode`
goldens — see Resolution and Validation Plan

## Resolution (2026-07-23)

All three structural causes are fixed and every touched surface is byte-identical
on all four registered targets (macos-aarch64, linux-aarch64, linux-x86_64,
linux-riscv64). Note the tree had already moved on from the `b12213d2` snapshot
this doc was written against — `tls/macos.rs` is now `tls/macos/{mod,client,
server,tests}.rs` (bug-327), `emit_alloc` was already promoted to `code/mod.rs`
(bug-322), and the `List OF Byte` build/read loops were already shared within
`crypto` (plan-57). The `.ncodesum` goldens the doc said did not exist now do
(plan-57's `cover-audio`/`cover-tls`/`cover-crypto`), so the artifact-gate is
real byte-identity coverage for this subtree.

What landed (one commit each):

1. **`native_helpers.rs`** — the package-neutral emitters (`hex_encode_cstring`,
   `emit_data_address`, `emit_arena_free`, `emit_fail`, `emit_read_byte_list`,
   `emit_build_byte_list`, `emit_zero_guarded`) moved out of `tls/mod.rs`;
   `tls`/`audio`/`crypto`/`crypto_ec` all import from it on equal terms. No
   module reaches into `tls` for a generic emitter any more. Collapsed the
   duplicates this exposed: crypto_ec's two `data_address`, two `call_fn`, and
   `zero_scratch_guarded`→generalized `zero_guarded`; `crypto.rs`'s
   `emit_arena_alloc` (a copy of `emit_alloc` bug-322 missed) and
   `emit_fail_result`; and `audio/alsa`'s `hex_encode_cstring`.
2. **TLS parent dispatch** — the seven `lower_tls_*_helper` entry points now
   dispatch in `tls/mod.rs` (mirroring `crypto_ec`); each backend file is a pure
   `*_openssl` / `*_macos` implementation. `Curve::bits`/`macos_algorithm` (macOS
   -only) moved into the macOS EC backend.
3. **`audio/common.rs` + `AudioBackend`** — the shared `enum Query`, the
   open-parameter ranges, `emit_validate_open`, and `READ_FRAMES_MAX` moved to
   `audio/common.rs`; an `AudioBackend` enum owns the data-object and
   AudioQueue-callback decisions, so `platform.target().contains("macos")` no
   longer appears in `code/mod.rs` for any audio decision and the callback
   symbol lists live next to the emitters they gate.
4. **`emit_port_itoa`** — the character-identical port-itoa loop factored across
   `tls/macos/{client,server}.rs`.
5. **EC `.ncode` goldens** — `crypto-ec-valid` gained its first artifact-level
   coverage (3 targets), since this refactor touched both EC backends heavily.

Deliberately left (as the doc's Blast Radius / Open Decisions anticipated):

- The `tls/macos` dispatch-deadline and 64-byte config-block intra-file dups
  (items #12/#13): the deadline block wraps a `dlsym` call and the config
  block's second half diverges — neither is the clean extraction `emit_port_itoa`
  was. File separately if wanted.
- `tls`'s own inline `List OF Byte` build loops (part of item #10): merging them
  onto `emit_build_byte_list` would churn a committed `.ncode` golden on label
  names for no module-home benefit.
- `crypto.rs`'s randomBytes zero loop kept inline (not routed through
  `emit_zero_guarded`) for the same reason — it would change only the dump label
  names, churning a committed golden with zero byte impact.
- The `net`/`link_thunk` copies, the ~77-site `emit_fail` epilogue table, and
  all man/spec drift — out of scope per the original Blast Radius.

Validation: `scripts/artifact-gate.sh` green (1321 goldens, 0 diffs — this now
includes the audio/tls/crypto/EC `.ncode` goldens); a scaffold per-target
`.ncode` harness over `cover-audio`/`cover-tls`/`cover-crypto`/`crypto-ec-valid`/
`accept_valid` green after every step; `cargo fmt --check`, `cargo clippy` (no
new warnings), and the tls/crypto/audio unit tests (14/8/2, incl. error-path
release tests) all pass. **Not** performed here: the real-hardware audio
playback/capture and live-TLS runtime checks the Validation Plan lists — those
need physical devices on both platforms. Byte-identity across all four targets
(the emitted machine-code stream is unchanged) is the standing proof that
runtime behavior is unchanged; the hardware checks remain advisable before a
release if any doubt remains.



Six files in `src/target/shared/code/` form three macOS/Linux backend pairs —
`audio/macos.rs`+`audio/alsa.rs`, `tls/macos.rs`+`tls/openssl.rs`,
`crypto_ec/macos.rs`+`crypto_ec/openssl.rs` (15,932 lines with their parents).
Each pair independently re-implements the same platform-neutral scaffolding:
arena allocation, `List OF Byte` marshalling, `dlopen`/`dlsym` sequencing,
data-symbol addressing, guarded key-material zeroing, monotonic-deadline
computation, and parameter validation. Several of these blocks are
character-identical across files; others differ only in a local label string or
a stack-slot constant name.

The duplication has a structural cause, not just copy-paste drift. Three
architectural facts make it self-perpetuating, and they are the real subject of
this bug:

1. **`tls` is the de-facto home for generic native-codegen helpers.**
   `audio/mod.rs:81` imports `emit_alloc`, `emit_arena_free`,
   `emit_data_address`, and `emit_fail` from `super::tls` — none of which have
   anything to do with transport-layer security. `tls/mod.rs:1-12`, the module
   doc, describes only OpenSSL and Network.framework; nothing in the module's
   name or documentation tells a reader that four of the compiler's generic
   emitters live there. A new backend author has no discoverable home for a
   shared emitter, so they write a local copy.
2. **`tls/openssl.rs` — nominally the OpenSSL backend — owns the macOS dispatch
   for all seven TLS helpers** (`:23`, `:850`, `:1369`, `:1722`, `:1981`,
   `:2128`, `:2293`), while `crypto_ec.rs:108` does exactly the same dispatch
   correctly in the shared parent. The repo already contains the right pattern
   immediately next to the wrong one.
3. **`audio/mod.rs` is not a platform-dispatch abstraction.** It dispatches once
   at `:113`, but the macOS-vs-Linux decision is re-derived three more times in
   `code/mod.rs` (`:641`, `:989`, `:1005`) via `platform.target().contains("macos")`
   plus hand-maintained literal symbol lists.

**The single correct outcome a fix produces:** every genuinely generic emitter
lives in one module named for what it does, each package's platform dispatch
happens exactly once in that package's parent module, and the emitted machine
code is byte-identical to today's for every target.

This is a maintainability bug, not a correctness bug. Nothing miscompiles today.
It is filed because the duplication is where future correctness bugs will land:
bug-116 (leaked `nw_release`) had to be fixed in two copies of the same
config-block builder inside a single file, and bug-55's key-zeroing requirement
is currently satisfied by three separate implementations under three names.

References:

- Cleanup review findings index, "Agent 07 — tls/crypto/link codegen" (#1, #2,
  #3, #9, #10, #11, #12) and "Agent 08 — audio/term codegen" (#1, #3, #10).
- `spec/architecture/06_native.md` — native runtime-helper family model.
- plan-33-A / plan-33-B (audio backends), plan-03-net.md §4 (TLS backends),
  plan-04-crypto.md Part C (EC backends).
- Related but out of scope: bug-300 (LOW docs/dead-code cluster).

## Current State

Every range below was diffed in the worktree at `main` (b12213d2). Line numbers
are as-committed.

### Measured duplication inventory

| # | Duplicated surface | Sites | Verified delta |
| --- | --- | --- | --- |
| 1 | `emit_alloc_byte_list` | `audio/macos.rs:1557-1611` vs `audio/alsa.rs:1181-1234` | 2 local label strings + 4 comments |
| 2 | `emit_validate_open` | `audio/macos.rs:127-161` vs `audio/alsa.rs:356-380` | offset params vs module consts + 3 comments |
| 3 | `enum Query` | `audio/macos.rs:1183-1188` vs `audio/alsa.rs:308-313` | **character-identical, 6 lines** |
| 4 | Open-parameter validation consts | `audio/macos.rs:99-102`,`:1551-1552` vs `audio/alsa.rs:316-321` | same 6 names, same 6 values |
| 5 | `×1_000_000` ns-deadline idiom | `audio/alsa.rs:1374`,`:1434`; `audio/macos.rs:1276`,`:2087`; `tls/macos.rs:877`,`:3044` | 6 sites; 5 are full absolute-deadline pins |
| 6 | `data_address` | `crypto_ec/macos.rs:74-109` vs `crypto_ec/openssl.rs:177-212` | **character-identical, 36 lines, same param names** |
| 7 | `call_fn` | `crypto_ec/macos.rs:198-204` vs `crypto_ec/openssl.rs:312-318` | **character-identical, 7 lines** |
| 8 | `emit_alloc` (child redefines parent's) | `crypto_ec/openssl.rs:376-394` vs `crypto_ec.rs:241-259` | identical modulo 2 parameter names |
| 9 | Guarded key-material zeroing | `crypto_ec/macos.rs:245-272` (`zero_scratch_guarded`), `crypto_ec/openssl.rs:441-478` (`zero_guarded`), `crypto.rs:180-198` (inline) | 3 impls, 3 names |
| 10 | `List OF Byte` build loop | `crypto_ec.rs:198-236`, `crypto.rs:134-172`, `tls/openssl.rs:1858-1895`, `tls/macos.rs:1236-1275` | 4 copies × ~39 lines; differ only in slot-const names and the local vec identifier |
| 11 | Port itoa loop (intra-file) | `tls/macos.rs:451-469` vs `tls/macos.rs:2530-2548` | **character-identical, 19 lines** |
| 12 | Dispatch-deadline (intra-file) | `tls/macos.rs:855-888` vs `tls/macos.rs:3022-3055` | 3 comment lines + 1 const name (`HANDLE`/`NWH`) |
| 13 | 64-byte config-block build (intra-file) | `tls/macos.rs:559-641` vs `tls/macos.rs:2372-2454` | ~83 lines; 8 differing lines (2 slot consts, 1 setter symbol, 1 comment word) |
| 14 | `hex_encode_cstring` | `tls/mod.rs:99-106` vs `audio/alsa.rs:68-75` | identical modulo 1 trailing comment |
| 15 | `emit_data_address` / `data_address` | `tls/mod.rs:153-183`, `crypto_ec/macos.rs:74-109`, `crypto_ec/openssl.rs:177-212` | the same adrp+add+2-reloc emitter, 3 times |

### The verbatim pair

`crypto_ec/macos.rs:74-109` and `crypto_ec/openssl.rs:177-212` are byte-for-byte
identical — same doc comment, same parameter names, same body:

```rust
/// Load the address of a read-only data symbol into `dst` (adrp + add).
fn data_address(
    from: &str,
    dst: &str,
    data_symbol: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", data_symbol),
    );
    ins.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", data_symbol),
    );
    rel.extend([
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}
```

`tls/mod.rs:153-183` is the same emitter a third time under the name
`emit_data_address`, differing only in visibility (`pub(super)`) and the
parameter names (`instructions`/`relocations`).

### Self-documenting evidence

Two comments in the tree already name the problem:

- `audio/mod.rs:79-81`:
  `// Shared emit helpers live in the `tls` module; reuse them rather than / // duplicating.` —
  followed by `pub(super) use super::tls::{emit_alloc, emit_arena_free, emit_data_address, emit_fail};`
  under an `#[allow(unused_imports)]`.
- `crypto.rs:177-179`: the inline zeroing loop's own comment says it is a
  *"Call-free guarded zero loop mirroring the EC helpers' `zero_scratch_guarded`"*.

### Corrections to the source findings

Three cited leads did not verify as stated and are recorded here so the
implementer does not chase them:

- **`dlsym_into` is NOT identical between the crypto_ec backends.**
  `crypto_ec/macos.rs:136-163` and `crypto_ec/openssl.rs:257-284` differ in one
  token: `data_address(symbol, abi::ARG[1], &sym(name), …)` vs `&fn_sym(name)`.
  They are a near-dup (1 differing line of 28), not a verbatim dup. Unifying
  them requires parameterizing the symbol-name mangler.
- **`zero_scratch_guarded` and `zero_guarded` are not two copies of one thing.**
  `crypto_ec/openssl.rs:441` is a strict generalization of
  `crypto_ec/macos.rs:245`: it takes `len_off: Option<usize>` plus a
  `len_const` fallback, of which the macOS version is exactly the `Some(off)`
  case. The OpenSSL signature is the correct one to promote.
- **The `tls/macos.rs` intra-file config-block dup is larger than cited.** The
  finding gave `:560-641` vs `:2372-2410`; the actual near-identical extent is
  `:559-641` vs `:2372-2454` (~83 lines each), and the second half of each block
  diverges into distinct parameter-configuration paths. The shared prefix is
  the block-literal construction; only that prefix is factorable.

Additionally, the "audio dispatch re-derived 3x" finding is precise about
`code/mod.rs` but should be read as **four** total derivation sites when
`audio/mod.rs:113` is counted; `code/mod.rs:2486` is a plain call into the
dispatcher, not a fourth derivation.

### Test coverage of the affected code — measured

This is the single most important fact for planning the refactor:

```
$ grep -rl "_mfb_rt_audio_"          tests/   →  (no matches)
$ grep -rl "_mfb_rt_tls_"            tests/   →  (no matches)
$ grep -rl "_mfb_rt_crypto_crypto_p256" tests/ →  (no matches)
$ find tests -name "*.ncode" | wc -l          →  6
```

No golden anywhere in `tests/` contains an `audio`, `tls`, or `crypto_ec`
runtime-helper symbol. The relevant test dirs carry only `.ast`, `.ir`, and
`.run` goldens:

```
tests/rt-behavior/tls/tls-connect-google-rt/golden/  → build.log  *.ast  *.ir  *.run
tests/rt-behavior/crypto/crypto-ec-valid/golden/     → build.log  *.ast  *.ir  *.run
tests/syntax/audio/open_invalid/golden/              → build.log
```

`.ast` and `.ir` dumps are produced **before** native lowering, so they are
entirely insensitive to any change in these six files. `scripts/artifact-gate.sh`
therefore provides **zero** protection for this refactor. There is no
`tests/rt-behavior/audio` directory at all — the audio backends have no runtime
test on either platform.

## Root Cause

Three seams that were never established:

1. **No module owns generic native-codegen emitters.** `tls/mod.rs` grew the
   first ones (`hex_encode_cstring:99`, `emit_data_address:153`,
   `emit_alloc:191`, `emit_arena_free:208`, `emit_fail:217`) because `tls` was
   the first `dlopen`-based package. `audio/mod.rs:81` then imported them from
   there rather than promoting them, and `crypto_ec` reached across two module
   levels (`super::super::tls::hex_encode_cstring` at `crypto_ec/macos.rs:56`
   and `crypto_ec/openssl.rs:147`) for the one helper it borrowed while
   hand-writing its own `emit_alloc` (`crypto_ec.rs:241`) and `data_address`
   (twice). The import path is the whole story: reusing a helper costs a
   confusing cross-package `use`, so copying wins.

2. **The TLS package has no parent-level dispatch.** `tls/mod.rs:401-408`
   re-exports the seven `lower_tls_*_helper` entry points *from `openssl`*, so
   `openssl.rs` must open each with `if platform.target().contains("macos") {
   return macos::…; }`. Because the OpenSSL file is the entry point, its local
   helpers are the ones in scope, and the macOS file cannot reuse them without
   an awkward sibling import — so `tls/macos.rs` grew its own full set. Compare
   `crypto_ec.rs:104-111`, where `lower_crypto_ec_helper` branches in the parent
   and both children are pure backends.

3. **`audio` has no backend abstraction, only a function.**
   `audio::lower_audio_helper` (`audio/mod.rs:104-118`) dispatches, but three
   other decisions in `code/mod.rs` — which data objects to emit (`:641`) and
   whether to emit the two AudioQueue callbacks (`:989`, `:1005`) — each
   re-derive the platform independently and gate on hand-written literal symbol
   lists (four symbols at `:990-997`, five at `:1006-1014`). Adding an audio
   helper means remembering to edit those lists; nothing enforces it. With no
   backend type to hang shared code on, `macos.rs` (2,884 lines) and `alsa.rs`
   (2,253 lines) each carry a full private skeleton.

The three causes compound: (1) removes the destination for shared code, (2)
removes the natural place to put a TLS-shared helper, and (3) removes the
audio equivalent. Every duplicate in the inventory is downstream of one of them.

## Goal

- One module, `src/target/shared/code/native_helpers.rs`, owns the generic
  emitters; `audio` and `crypto_ec` no longer import anything from `tls`.
- Each of `tls`, `crypto_ec`, and `audio` performs its macOS/Linux dispatch
  exactly once, in the package's own `mod.rs`/parent.
- `platform.target().contains("macos")` appears zero times in `code/mod.rs` for
  audio decisions.
- The inventory items above collapse to one implementation each.
- **Generated machine code is byte-identical** for macos-aarch64,
  macos-x86_64, linux-aarch64, linux-x86_64 (glibc and musl), and
  linux-riscv64, for every program that uses `audio`, `tls`, or `crypto`.

### Non-goals (must NOT change)

- **Emitted bytes.** Not one instruction, relocation, data object, stack-slot
  offset, or frame size may change on any target. This is the hard constraint;
  every phase is gated on it.
- **Emitted label names.** Labels do not affect encoded bytes, but they *do*
  appear in `.nplan`/`.nobj`/`.ncode` dumps. Item #1 (`emit_alloc_byte_list`)
  differs precisely in two label strings; unifying it changes ALSA's dump text.
  Pick one spelling, accept the dump delta, and confirm the encoded bytes are
  unchanged — do not treat a dump diff as proof of a code change, and do not
  "fix" it by keeping both spellings behind a flag.
- Runtime behavior of `audio`, `tls`, or `crypto` on either platform.
- The `AudioHandle`/`AudioState`/`TlsSocket` record layouts and every constant
  in `audio/mod.rs:17-77`, `tls/mod.rs:19+`.
- The public `RuntimeHelper` surface, the runtime-spec catalog, `.mfp` format.
- Man pages and specs — the several man-drift findings in the same review
  (Agent 07 #17–#21) are separate bugs; do not fold them in here.
- **Forbidden shortcut:** do not "prove" byte-identity by regenerating goldens
  and observing they match. As measured above, no golden covers this code. A
  regenerate-and-diff run is vacuous here and must not be presented as
  validation.

## Blast Radius

Found by `grep -rn 'tls::' src/target/shared/code/` and
`grep -rn 'contains("macos")' src/target/shared/code/`.

Fixed by this bug:

- `audio/mod.rs:81` — imports 4 generic emitters from `tls`. Repointed to
  `native_helpers`.
- `crypto_ec/macos.rs:56`, `crypto_ec/openssl.rs:147` — reach `tls` via
  `super::super::tls::hex_encode_cstring`. Repointed.
- `audio/alsa.rs:68-75` — local `hex_encode_cstring` copy. Deleted.
- `tls/mod.rs:99,153,191,208,217` — the five generic emitters. Moved out;
  `tls` re-imports them like any other consumer.
- `crypto_ec.rs:241` + `crypto_ec/openssl.rs:376` — two `emit_alloc`. Both
  deleted in favor of the shared one.
- `crypto_ec/macos.rs:74`, `crypto_ec/openssl.rs:177` — two `data_address`.
  Deleted.
- `crypto_ec/macos.rs:198`, `crypto_ec/openssl.rs:312` — two `call_fn`. Merged.
- `crypto_ec/macos.rs:245`, `crypto_ec/openssl.rs:441`, `crypto.rs:180-198` —
  three guarded-zero impls. Collapsed onto the OpenSSL signature.
- `crypto_ec.rs:198-236`, `crypto.rs:134-172`, `tls/openssl.rs:1858-1895`,
  `tls/macos.rs:1236-1275` — four byte-list build loops. All call
  `crypto_ec.rs:176`'s `emit_build_byte_list`, promoted to `native_helpers`.
- `tls/openssl.rs:23,850,1369,1722,1981,2128,2293` — seven macOS dispatch
  branches. Moved to `tls/mod.rs`.
- `tls/mod.rs:401-408` — re-export block. Replaced by real dispatchers.
- `crypto_ec.rs:42-57` (`Curve::bits`, `Curve::macos_algorithm`) — macOS-only
  data in the shared parent; only callers are `crypto_ec/macos.rs:499,914,1219`.
  Moved into `crypto_ec/macos.rs`. `Curve` itself stays in the parent (the
  OpenSSL backend has its own `params()` table at `crypto_ec/openssl.rs:81`).
- `audio/macos.rs` + `audio/alsa.rs` inventory items #1–#5 — moved to
  `audio/common.rs`.
- `audio/mod.rs:113` and `code/mod.rs:641,989,1005` — collapsed behind an
  `AudioBackend` dispatch.
- `tls/macos.rs:451-469`/`:2530-2548`, `:855-888`/`:3022-3055`,
  `:559-641`/`:2372-2454` — three intra-file dups. Factored within the file.

Latent, same hazard, **out of scope**:

- `net/mod.rs:45,61` and `link_thunk.rs:80,109` carry their own copies of the
  same alloc/data-address shapes. Out of scope because `net` and `link_thunk`
  are not platform-backend pairs — they belong with the broader
  "50 byte-identical arena-alloc blocks" finding (Agent 01 #2 / Agent 02 #4),
  which proposes a `CodeBuilder::emit_arena_alloc` method covering ~150
  repo-wide sites. This bug must not pre-empt that design; `native_helpers.rs`
  should be shaped so those sites can migrate onto it later.
- The ~77 `label + emit_fail` epilogue blocks across the four TLS/EC files and
  the 30 audio error epilogues (Agent 07 #4, Agent 08 #2). Genuinely the largest
  mechanical reduction available, but it is a different refactor (a fail-table
  driven by a slice literal) and would obscure the diff of this one. File
  separately.
- `tls/macos.rs`'s 3,960-line size and its private `TlsReadTestPlatform`
  (Agent 07 #5, #8). File-splitting is a prerequisite-free follow-up; doing it
  in the same change makes byte-identity impossible to review.

Unaffected:

- `arch/` encoders — this refactor operates entirely above the MIR/CodeOp layer
  and emits the same `CodeInstruction` stream.
- All non-`dlopen` runtime-helper families (`fs`, `io`, `os`, `strings`,
  collections) — they do not import from `tls` and have no backend pair.

## Fix Design

Four steps in strict dependency order. **(1) and (2) are prerequisites for
(3) and (4)**: until the generic emitters have a home and TLS dispatches from
its parent, there is nowhere to put `audio/common.rs`'s shared skeleton that
does not re-create the `audio → tls` dependency this bug exists to remove.

### (1) `src/target/shared/code/native_helpers.rs` — the generic emitter home

New module, declared in `code/mod.rs`, owning exactly the emitters that have no
package affinity:

- `hex_encode_cstring` (from `tls/mod.rs:99`)
- `emit_data_address` (from `tls/mod.rs:153`; absorbs both `crypto_ec`
  `data_address` copies verbatim)
- `emit_alloc` (from `tls/mod.rs:191`; absorbs `crypto_ec.rs:241` and
  `crypto_ec/openssl.rs:376`)
- `emit_arena_free` (from `tls/mod.rs:208`)
- `emit_fail` (from `tls/mod.rs:217`; `crypto_ec.rs:261` is the same shape)
- `emit_build_byte_list` (promoted from `crypto_ec.rs:176`, the existing
  factored version) and `emit_read_byte_list` (`crypto_ec.rs:~118`)
- `emit_zero_guarded` — the `crypto_ec/openssl.rs:441` generalization
  (`len_off: Option<usize>` + `len_const`), which subsumes the macOS and
  `crypto.rs` copies

Every one of these moves is a cut-and-paste plus an import rewrite: the bodies
already emit identical instruction sequences, so byte-identity is structurally
guaranteed for this step *except* where a caller's local label spelling differs
(item #1, and the `crypto.rs:180` inline zero loop's `_zero_skip`/`_zero_loop`/
`_zero_end` vs the helper's `_noz`/`_zl`/`_ze`). Land those label renames
deliberately and note the dump delta.

`tls/mod.rs` then imports from `native_helpers` exactly as `audio` and
`crypto_ec` do — no module is privileged. Update `tls/mod.rs:1-12`'s doc so it
describes TLS only.

*Rejected:* putting the helpers on the `CodegenPlatform` trait
(`types.rs`). It is already a 65-method god-trait (Agent 04 #5); these are free
functions over `Vec<CodeInstruction>` with no platform dependency, and adding
them would make the trait harder to split later.

*Rejected:* `pub use`-ing them from `tls` to avoid touching call sites. That
preserves the exact confusion this bug is about.

### (2) Move the macOS dispatch out of `tls/openssl.rs` into `tls/mod.rs`

Mirror `crypto_ec.rs:104-111` exactly. `tls/mod.rs` gains seven small
dispatchers:

```rust
pub(super) fn lower_tls_connect_helper(…) -> … {
    if platform.target().contains("macos") {
        return macos::lower_tls_connect_macos(symbol, platform_imports, platform);
    }
    openssl::lower_tls_connect_openssl(symbol, platform_imports, platform)
}
```

`tls/openssl.rs`'s seven entry points lose their leading `if` (`:23`, `:850`,
`:1369`, `:1722`, `:1981`, `:2128`, `:2293`) and are renamed `*_openssl` to
match the macOS side's existing `*_macos` naming. The re-export block at
`tls/mod.rs:401-408` is deleted, and the stray `tls.connect` banner at
`tls/mod.rs:397-399` — which introduces module declarations, not connect code —
goes with it.

Purely mechanical: the same branch runs, one frame higher. Zero byte impact.

### (3) `src/target/shared/code/audio/common.rs`

Now that (1) exists, the shared audio skeleton has somewhere to live that does
not route through `tls`. Move in, in this order (smallest byte-risk first):

- `enum Query` (item #3 — character-identical; zero risk)
- The six validation constants (item #4 — same values; zero risk)
- `emit_validate_open` (item #2) — take the three offsets as parameters, i.e.
  the macOS signature; ALSA's call site passes its `SR_OFF`/`CH_OFF`/`BF_OFF`
  constants. Emits the same instructions.
- `emit_alloc_byte_list` (item #1) — the one place a visible delta lands. Pick
  the ALSA label spelling (`_bl`/`_bl_done`, the shorter) or the macOS one, and
  record which in the commit message.
- `emit_monotonic_deadline` (item #5) — parameterize the `CLOCK_MONOTONIC`
  value (6 on macOS, 1 on Linux) and the destination slot. This covers the four
  audio sites; the two `tls/macos.rs` sites use `dispatch_time` rather than
  `clock_gettime` and are handled by (5) below, not shared with audio.

### (4) A real `AudioBackend` dispatch in `audio/mod.rs`

Introduce a small enum (or trait object) selected once from `platform`:

```rust
enum AudioBackend { CoreAudio, Alsa }
fn audio_backend(platform: &dyn CodegenPlatform) -> AudioBackend { … }
```

with methods for the three decisions `code/mod.rs` currently makes inline:
`data_objects()`, `callback_functions(runtime_symbols)`, and
`lower_helper(call, …)`. The hand-maintained symbol lists at `code/mod.rs:990-997`
and `:1006-1014` move next to the callback emitters they gate, in `audio/macos.rs`.
`code/mod.rs:641`, `:989`, `:1005` each collapse to one call. The
`#[allow(unused_imports)]` at `audio/mod.rs:80` is dropped (all four imports are
used).

*Rejected:* a full `dyn` trait with a Linux and a macOS impl struct. The two
backends' entry points already differ in signature (the callbacks are
macOS-only), so a trait would need `Option`-returning methods on one side; an
enum with three methods is smaller and equally effective at removing the
re-derivation.

### (5) Intra-file `tls/macos.rs` factoring (independent; land last)

Three local extractions, none of which touch another file:

- `emit_port_itoa` — `:451-469` and `:2530-2548` are character-identical.
- `emit_dispatch_deadline` — `:855-888` vs `:3022-3055`; parameterize the
  handle slot.
- `emit_cfg_block` — the shared prefix of `:559-641` and `:2372-2454`;
  parameterize the captured-payload slot and the setter symbol name. The
  existing `emit_build_block` handles only the 40-byte 1-capture shape, so this
  needs a 64-byte 4-capture variant rather than an extension of it.

## Phases

### Phase 1 — establish a byte-identity harness (no source change)

Because no golden covers this code (see Current State), the refactor needs a
harness built for it before anything moves.

- [ ] Write ~6 fixture programs under `tests/` — one per affected surface:
      `audio` open/write/read/close, `tls` connect and listen/accept, `crypto`
      `randomBytes`, and `crypto` P-256 generate/sign/verify — chosen so their
      plans pull in every helper in the Blast Radius.
- [ ] For each, capture `-ncode` (and `-nobj`) on **every** target the toolchain
      can cross-emit: macos-aarch64, macos-x86_64, linux-aarch64, linux-x86_64
      (glibc **and** musl), linux-riscv64. Store as a baseline outside `tests/`
      (this is a scaffold, not a committed golden).
- [ ] Confirm the baseline is reproducible: capture twice, diff, expect empty.
      A non-empty diff means codegen nondeterminism (cf. bug-87) and must be
      resolved before proceeding.
- [ ] Decide and record whether any of these fixtures should become permanent
      `.ncode` goldens. Recommendation: yes for the `crypto` EC one, which is
      network-free and deterministic.

Acceptance: a script that re-emits all fixtures on all targets and diffs against
the baseline, green on an unmodified tree.
Commit: —

### Phase 2 — (1) `native_helpers.rs`, then (2) TLS parent dispatch

- [ ] Create `native_helpers.rs`; move the seven generic emitters; rewrite the
      imports in `tls/mod.rs`, `audio/mod.rs:81`, `crypto_ec.rs`,
      `crypto_ec/macos.rs:56`, `crypto_ec/openssl.rs:147`; delete
      `audio/alsa.rs:68-75`, `crypto_ec.rs:241`, `crypto_ec/openssl.rs:376`,
      and both `data_address` copies.
- [ ] Point all four byte-list build loops at `emit_build_byte_list`; collapse
      the three guarded-zero impls onto `emit_zero_guarded`.
- [ ] Move the seven macOS dispatch branches from `tls/openssl.rs` to
      `tls/mod.rs`; delete the re-export block and the misplaced banner.
- [ ] Move `Curve::bits`/`Curve::macos_algorithm` from `crypto_ec.rs:42-57`
      into `crypto_ec/macos.rs`.
- [ ] Run the Phase 1 harness after **each** bullet, not once at the end. Any
      diff other than a deliberately recorded label rename is a defect.

Acceptance: harness green on all six targets; the only accepted deltas are the
enumerated label renames, each with a written justification.
Commit: —

### Phase 3 — (3) `audio/common.rs`, (4) `AudioBackend`, (5) `tls/macos.rs`

- [ ] Create `audio/common.rs`; move items #3, #4, #2, #1, #5 in that order,
      running the harness between each.
- [ ] Introduce `AudioBackend`; collapse `code/mod.rs:641`, `:989`, `:1005`;
      move the two literal symbol lists into `audio/macos.rs`; drop the
      `#[allow(unused_imports)]` at `audio/mod.rs:80`.
- [ ] Extract `emit_port_itoa`, `emit_dispatch_deadline`, and `emit_cfg_block`
      inside `tls/macos.rs`.
- [ ] `cargo fmt` (a second pass in `repository/`, which is not a workspace
      member) and `cargo clippy`.

Acceptance: harness green; `grep -rn 'contains("macos")' src/target/shared/code/mod.rs`
returns nothing audio-related; `grep -rn 'tls::' src/target/shared/code/audio
src/target/shared/code/crypto_ec*` returns nothing.
Commit: —

### Phase 4 — full validation, including the runtime checks the harness cannot give

- [ ] `scripts/artifact-gate.sh` — cheap and fast, but see the note in
      Validation: it does **not** cover this code. Run it to catch collateral
      damage elsewhere, not as evidence for this change.
- [ ] `scripts/test-accept.sh` — full acceptance on macOS and on Linux.
- [ ] The real runtime checks (below), on real hardware.

Acceptance: full suite green on both platforms; every runtime check observed,
not inferred.
Commit: —

## Validation Plan

- **Regression test(s):** none in the failing-then-passing sense — this is
  behavior-preserving. The substitute is the Phase 1 byte-identity harness,
  plus (recommended) one permanent `.ncode` golden for the `crypto` EC fixture,
  which would give this code its first artifact-level coverage.

- **`scripts/artifact-gate.sh`:** run it, but understand what it proves here.
  It regenerates `-ast -ir -br -nir -nplan -nobj -ncode` dumps for every
  `tests/**/project.json` that has a `golden/` dir and diffs them. As measured
  in Current State, **no golden in the tree contains an `audio`, `tls`, or
  `crypto_ec` runtime-helper symbol**, and the affected test dirs carry only
  `.ast`/`.ir`/`.run` — dumps produced before native lowering. The gate is
  therefore a check that this refactor did not perturb *unrelated* codegen. It
  is not, and must not be reported as, evidence of byte-identity for the six
  files this bug touches.

- **`scripts/test-accept.sh`:** the full acceptance harness. It runs `.run`
  goldens, which do execute the affected paths for
  `tests/rt-behavior/crypto/crypto-ec-valid` and
  `tests/rt-behavior/tls/tls-connect-google-rt`. Note that the latter is
  network-dependent (it connects to a live host), so a green run needs
  connectivity and a red run needs triage before it is believed.

- **Runtime proof the golden harness cannot give — flag these explicitly:**

  | Surface | Provable by goldens? | Required real check | Platform |
  | --- | --- | --- | --- |
  | `crypto` EC generate/sign/verify | partly — `.run` golden exists | run `crypto-ec-valid`; additionally verify cross-platform wire compat (sign on macOS, verify on Linux) since both backends were touched | macOS **and** Linux |
  | `crypto.randomBytes` | `.run` golden exists | run it; confirm the scratch-wipe still happens (the inline zero loop is being replaced) | either |
  | `tls.connect` | `.run` golden exists but is network-dependent | real TLS connect to a live host on **both** backends — Network.framework on macOS, OpenSSL on Linux | macOS **and** Linux |
  | `tls.listen`/`accept`/`write`/`close` | **no** | hand-run a listen/accept/echo program against a real client; exercise the timeout path (the dispatch-deadline extraction touches it) | macOS **and** Linux |
  | `audio` open/write/close (output) | **no — no test exists at all** | play real PCM through a real device and listen | macOS (AudioQueue) **and** Linux (ALSA) |
  | `audio` open/read/close (input) | **no — no test exists at all** | capture from a real device; verify frame counts and the timed-read partial-return path | macOS **and** Linux |

  The audio row is the sharp edge: `audio/macos.rs` and `audio/alsa.rs` total
  5,137 lines with **zero** test coverage of any kind, and `audio/common.rs`
  touches both. Do not land Phase 3 without hardware playback and capture on
  both platforms. plan-33 was originally validated this way ("macOS AudioQueue
  DONE+HW-VERIFIED"); the same bar applies.

- **Doc sync:** rewrite `tls/mod.rs:1-12` to describe TLS only (it currently
  sits above four generic emitters); give `native_helpers.rs` and
  `audio/common.rs` module docs. Check whether
  `spec/architecture/06_native.md` names `tls` as a helper location. The audio
  and TLS man/spec drift items from the same review (Agent 07 #17–#21,
  Agent 08 #11–#14) are **separate bugs** — do not fold them in.

- **Full suite:** `scripts/test-accept.sh` on macOS and Linux, plus
  `cargo test`, `cargo fmt --check`, `cargo clippy`.

## Open Decisions

- **Which label spelling wins in `emit_alloc_byte_list`** — recommend ALSA's
  (`{symbol}_{tag}_bl` / `_bl_done`, shorter, and ALSA has no `.nplan`/`.nobj`
  goldens to churn). Alternative: macOS's `_bl_entry`/`_bl_entry_done`. Either
  way one backend's dump text changes and the bytes do not. (§Non-goals)
- **Should the Phase 1 fixtures become committed goldens?** Recommend yes for
  the `crypto` EC fixture (deterministic, network-free) — it would be the first
  artifact-level coverage this subtree has ever had, and it makes the *next*
  refactor cheap. Alternative: keep all fixtures as scaffolding and delete them,
  leaving coverage at zero. (§Phase 1)
- **`AudioBackend` as enum vs. trait object** — recommend enum (see Fix Design
  (4)). Alternative: `dyn AudioBackend` for symmetry with a future
  `TlsBackend`/`EcBackend`; defer until a second package needs it. (§Fix Design)
- **Whether to also collapse the ~77 `emit_fail` epilogues now** — recommend no;
  it is the largest single reduction available (~550 lines) but it would bury
  this refactor's diff. File separately once `native_helpers.rs` exists, since
  the fail-table helper belongs there. (§Blast Radius)

## Summary

The engineering risk is **not** in the refactor's mechanics — most of the
inventory is cut-and-paste of already-identical code — but in the fact that
**this subtree has no automated coverage of its emitted bytes**. Measured:
zero goldens contain an `audio`, `tls`, or `crypto_ec` runtime-helper symbol,
and `audio` has no runtime test on either platform. So the byte-identity
harness in Phase 1 is the load-bearing work; everything after it is
straightforward if that harness is trustworthy, and unsafe if it is not.

Steps (1) `native_helpers.rs` and (2) TLS parent dispatch are prerequisites —
they create the destination and remove the `audio`/`crypto_ec` → `tls`
dependency that makes copying cheaper than sharing. (3) and (4) are only
worthwhile once that dependency is gone.

Left untouched: all emitted bytes, all record layouts, all runtime behavior,
the `net`/`link_thunk` copies of the same shapes (they belong to the broader
repo-wide arena-alloc consolidation), the ~77-site `emit_fail` epilogue
reduction, the `tls/macos.rs` file split, and every man/spec drift item found
alongside these findings.
