# plan-126-B: The `mfb_wire` crate — byte primitives and MFP framing

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-A (letter order only; no code dependency)

Creates the third workspace crate and moves into it the byte-level decoding
primitives and `.mfp` container framing that the compiler and the registry each
implement today. After this sub-plan there is exactly one implementation of "read a
length-prefixed string from a byte cursor" and one definition of `MFP_MAGIC` in the
tree, and the registry can decode container framing without restating the
compiler's constants.

Behavioral outcome: **nothing observable changes.** Every `.mfp` this compiler
writes is byte-identical to before, every error string is unchanged, and both MFP
header decoders keep their own guard sets. This is pure code motion, and that is
what makes it safe to land alone.

References:

- `src/binary_repr/mod.rs:25-31` — the bug-340 B8 note explaining why the two MFP
  decoders are deliberately **not** merged. This sub-plan honors it; read it first.
- `repository/src/abi.rs:1-8` — the registry's own statement of the dependency
  problem this crate solves.
- `repository/Dockerfile:22-35` — the deploy image's workspace handling.
- `.ai/build-tooling.md` — rustfmt policy (and one stale line this sub-plan fixes).

## Prerequisites

See plan-126-A § Prerequisites — they govern the whole plan-126 feature.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- A new workspace member `mfb_wire` (directory `wire/`) with **no dependency on
  either `mfb` or `mfb_repository`**, and no I/O, HTTP, database or async
  dependencies — `sha2` only.
- `src/binary_repr/util.rs` lives in `mfb_wire`; the compiler's `binary_repr`
  re-exports it so its 200+ internal call sites are unchanged.
- `MFP_MAGIC`, the `.mfp` fixed-prefix field readers, and the signature-header rule
  have one home in `mfb_wire`; both `src/manifest/package.rs:read_mfp_header` and
  `repository/src/package.rs:parse_mfp_package` are built on it while keeping their
  own, different, guard sets.
- `repository/Dockerfile` still builds `mfb-repo` without compiling the compiler.

### Non-goals (explicit constraints)

- **The two MFP header decoders are NOT merged.** `src/binary_repr/mod.rs:25-31`
  records that decision (bug-340 B8): the manifest reader additionally enforces
  per-field byte limits, UTF-8, required-non-empty and `validate_package_name`, and
  returns fields the identity/payload decoder omits. Folding them would drop
  trust-boundary guards. Share the primitives; keep two named policies.
- **No behavior change of any kind.** Same error strings, same accept/reject
  decisions on both sides, byte-identical `.mfp` output.
- **`crypto`, `MfpPackage`, the `server` wire DTOs, `client` and `local` do not
  move.** They stay in `mfb_repository`; `mfb` keeps depending on it for them. This
  scope was decided explicitly — the goal is one home for what is *duplicated*, not
  a full re-layering.
- No change to the `.mfp` or MFPC wire format, section ids, or field ordering.

## 2. Current State

**Dependency direction.** Root `Cargo.toml` declares
`mfb_repository = { path = "repository" }` and `[workspace] members = [".",
"repository"]` with the same `default-members`. So `mfb_repository` cannot reference
`mfb`, which `repository/src/abi.rs:1-8` and `src/terminal_safe.rs:1-11` both state
in prose as the reason they restate or re-export things.

**The deploy image depends on the compiler *not* being built.**
`repository/Dockerfile:28` writes a stub:

```dockerfile
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs
```

with the comment "`mfb` is a workspace member, so cargo must be able to resolve its
manifest — but it is never built here." A third member must be `COPY`d for real, not
stubbed, since `mfb_repository` will genuinely depend on it.

**`util.rs` is the compiler's byte-primitive layer**, 304 lines / 28 functions
(`grep -n "^pub(super) fn" src/binary_repr/util.rs` → 28). It has no compiler-specific
dependencies: `use super::*` pulls in `ABI_HASH_LEN` (`src/binary_repr/mod.rs:100`),
`Section` (`src/binary_repr/mod.rs:1133-1136`) and `Sha256`. Nothing from `crate::ir`,
`crate::ast`, `crate::types` or `crate::manifest`.

**Every `binary_repr` submodule imports it by glob.** `reader.rs:1`, `writer.rs:1`,
`sections.rs:1` and `builder.rs:1` all begin `use super::*;`, and `mod.rs:21` has
`use util::*;`. Re-pointing that one glob is what makes this move cheap.

**The `.mfp` fixed prefix is parsed twice**, field-for-field:

| | compiler | registry |
|---|---|---|
| entry point | `src/manifest/package.rs:71 read_mfp_header` | `repository/src/package.rs:99 parse_mfp_package` |
| magic | `crate::binary_repr::MFP_MAGIC` | literal array, `repository/src/package.rs:3` |
| `read_mfp_string` | `:169` | `:324` |
| `read_mfp_bytes` | `:184` | `:339` |
| `read_u16` / `read_u32` | `:211` / `:218` | `:421` / `:428` |
| signature-header rule | `validate_mfp_signature_header` (`src/binary_repr/mod.rs:32`) | `validate_signature_header` (`repository/src/package.rs:315`) |
| `ident` field | read `required=false` | read `required=true` |
| name charset guard | calls `validate_package_name` | none |

The last two rows are the *policy* difference bug-340 B8 protects. They stay.

### Measured populations

| What | Count | Command |
|---|---|---|
| `util.rs` lines / functions | 304 / 28 | `wc -l src/binary_repr/util.rs`; `grep -c "^pub(super) fn" src/binary_repr/util.rs` |
| `cursor_u32` call sites in `binary_repr` | 128 | `grep -rho cursor_u32 src/binary_repr --include='*.rs' \| wc -l` |
| `bounded_capacity` | 29 | same form |
| `cursor_u16` | 22 | same form |
| `cursor_string` | 17 | same form |
| `cursor_optional_str` / `cursor_prose_list` / `cursor_pair_list` | 7 / 6 / 5 | same form |
| `util_tests.rs` tests / lines | 17 / 274 | `grep -c '#\[test\]' src/binary_repr/tests/util_tests.rs`; `wc -l` |
| `MFP_MAGIC` references — compiler / registry | 15 / 3 | `grep -rho MFP_MAGIC src --include='*.rs' \| wc -l`; same over `repository/src` |
| `read_mfp_header` references in `src/` | 42 | `grep -rho read_mfp_header src --include='*.rs' \| wc -l` |
| `binary_repr` total lines | 5,949 | `wc -l src/binary_repr/*.rs` |
| Committed `.mfp` fixtures under `tests/` | 159 | `git ls-files '*.mfp' \| wc -l` |

### Verified properties

- **`util.rs` is compiler-independent.** Read all 28 functions: the only non-`std`
  reference is `Sha256::digest` in `hash_bytes` (`src/binary_repr/util.rs:85-91`)
  and the `ABI_HASH_LEN`/`Section` types, both plain data. `repository/Cargo.toml`
  already declares `sha2 = "0.10"`, so no new dependency enters the deploy image.
- **The glob-import structure holds.** Verified by reading line 1 of `reader.rs`,
  `writer.rs`, `sections.rs`, `builder.rs` — all `use super::*;`. So a
  `pub use mfb_wire::bytes::*;` in `src/binary_repr/mod.rs` reaches all four
  submodules without touching a single call site.
- **`packages/*.mfp` are build artifacts, not committed** (`git ls-files
  'packages/*.mfp'` → no output), but **159 `.mfp` fixtures under `tests/` are
  committed** and are consumed by `rt-behavior` tests. Those are the real
  regression detector for a framing change: a decoder that shifts by one byte fails
  them.
- **`.ai/build-tooling.md:21` is stale.** It states "There is no `[workspace]`
  table in the root `Cargo.toml`", but the table exists (added by bug-347) and
  `cargo metadata --no-deps` reports 2 workspace members. This sub-plan adds a third
  member, so the line must be corrected rather than left to mislead.

## 3. Design Overview

```
        mfb_wire  (new: byte primitives + MFP/MFPC framing; sha2 only)
           ↑                              ↑
           |                              |
    mfb_repository  ←──────────────────  mfb
    (server + client)                 (compiler)
```

`mfb` keeps depending on `mfb_repository` for `client`/`local`/`crypto`/`package` —
that arrow is correct, since the compiler genuinely *is* a registry client. The new
crate only removes the arrow that could not exist.

Module layout inside `mfb_wire`:

- `wire/src/bytes.rs` — the former `src/binary_repr/util.rs`, `pub`.
- `wire/src/mfp.rs` — `MFP_MAGIC`, `FIXED_PREFIX_LEN`, `read_mfp_string`,
  `read_mfp_bytes`, `read_u16`/`read_u32`/`read_u64`, and the signature-header rule.
- `wire/src/lib.rs` — module declarations and the crate doc explaining the
  dependency rule that justifies its existence.

**Where correctness risk concentrates:** the MFP framing move (Phase 3). Both
decoders read the same fixed prefix by advancing a shared offset; an off-by-one in a
shared field reader breaks package identity verification on the registry *and*
`mfb pkg` on the client. That is why it lands last, behind the 159 committed `.mfp`
fixtures.

**Where design uncertainty concentrates:** whether the Docker build still works with
a third member. Cheap to falsify — build the image — so it is Phase 1's acceptance,
before any code moves.

**Byte-identity IS this sub-plan's gate.** This is provably-neutral code motion whose
whole point is to preserve output exactly, which is the one class where byte-identity
is the right, strongest check. `scripts/artifact-gate.sh target/release/mfb all` must
report `diffs=0`, and no `.ncodesum` golden may be regenerated. A diff means a bug was
introduced in the move — objdump one fixture, localize it, fix it. **It never means
the design is dead.**

**Rejected alternative — move only the seven doc-table cursor helpers.** Measured,
`cursor_u32` alone has 128 call sites inside `binary_repr` and the helpers call each
other (`cursor_prose_list` → `cursor_u32` + `bounded_capacity` + `cursor_string`,
`src/binary_repr/util.rs:30-44`). Splitting the file leaves two homes for one layer —
the exact problem being fixed.

**Rejected alternative — invert the dependency so `mfb_repository` depends on `mfb`.**
The compiler uses five repository modules across 10 files (client 52, package 35,
crypto 25, server 18, local 11 references), all of which would have to move into the
compiler; and `mfb-repo`'s deploy image would then compile all of codegen, `image`
and `icns`, defeating `repository/Dockerfile:28`.

## Compatibility / Format Impact

Nothing externally observable changes. The `.mfp` and MFPC wire formats, section
ids, field order, error strings and public APIs of both existing crates are
unchanged. The only new public surface is the `mfb_wire` crate itself, which no
external consumer sees.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Empty crate, wired into the workspace and the deploy image

Proves the riskiest unknown (the Docker build) before any code moves.

- [x] Create `wire/Cargo.toml` (`name = "mfb_wire"`, edition 2021, `sha2 = "0.10"`,
      and the same `[lints.clippy] items_after_test_module = "deny"` both existing
      crates carry) and `wire/src/lib.rs` with a crate doc stating the dependency
      rule: `mfb_wire` depends on neither sibling; both siblings depend on it.
      The crate doc also names the three prose apologies this crate retires and
      records the bug-340 B8 rule that sharing primitives is not merging policies.
- [x] Add `"wire"` to `[workspace] members` **and** `default-members` in the root
      `Cargo.toml`, so a bare `cargo test` still means the whole workspace
      (the reason bug-347 named both).
- [x] Add `mfb_wire = { path = "../wire" }` to `repository/Cargo.toml` and
      `mfb_wire = { path = "wire" }` to the root `Cargo.toml`.
- [x] Add `COPY wire/Cargo.toml ./wire/` and `COPY wire/src ./wire/src` to
      `repository/Dockerfile`. The `mfb` stub stays, and its comment now says
      why `wire` gets no stub: unlike `mfb` it is a real dependency of
      `mfb_repository`, so it is compiled for real.
- [x] Correct the stale workspace claim in `.ai/build-tooling.md` — but **not**
      as the plan predicted. See Corrections: the claim that `cargo fmt --all`
      does not reach `repository/` is *also* false now, which I established by
      probe rather than by assuming the surrounding text was right.

Acceptance: MET.
`rustup run 1.96.0 cargo metadata --no-deps --format-version 1` → **3** workspace
members (`mfb`, `mfb_repository`, `mfb_wire`).
`docker build --load -f repository/Dockerfile -t mfb-repo-p126:test .` → exit 0,
with `Compiling mfb_wire v0.1.0 (/build/wire)` then `Compiling mfb_repository`
inside the builder stage (so the third member is genuinely compiled, not
stubbed), and `docker run --entrypoint /usr/local/bin/mfb-repo mfb-repo-p126:test`
prints the real usage banner. Note a bare `docker build` under a buildx driver
does **not** load the image into the local store — `--load` is required before
`docker run` can see it, which is why the first verification attempt reported
"Unable to find image".
Commit: baf71a193 (with Phase 2)

### Phase 2 — Move the byte primitives

- [x] Move `src/binary_repr/util.rs` to `wire/src/bytes.rs` (via `git mv`, so the
      rename is visible in history), changing only `pub(super)` → `pub` on all 29
      items and replacing `use super::*;` with an explicit
      `use sha2::{Digest, Sha256};`. Added a module doc naming the three guard
      classes that run through the file (`checked_add` per PKG-07,
      `checked_usize`, `bounded_capacity` per PKG-05) — they were unexplained at
      the file level and each one is a rejected `.mfp` away from a crash.
- [x] Move `ABI_HASH_LEN` and `Section` into `wire/src/bytes.rs`, since
      `hash_bytes`, `cursor_hash`, `hex_hash` and `encode_sections` need them.
      **Also `MFPC_MAJOR_VERSION`**, which `encode_sections` stamps — the plan
      missed it. See Corrections.
- [x] In `src/binary_repr/mod.rs`, delete `mod util;` / `use util::*;` and add
      `pub(crate) use mfb_wire::bytes::*;`. Verified the four submodules still
      resolve via their existing `use super::*;`.
- [x] Move the tests from `src/binary_repr/tests/util_tests.rs` into
      `wire/src/bytes.rs`'s own test module. **14 of the 17 moved, not all 17** —
      three were misfiled in that file and stayed in the compiler. See
      Corrections. Deleted the file and its `mod util_tests;` line.
- [x] Check the deletion did not orphan a doc comment onto a neighbouring item in
      `src/binary_repr/mod.rs` and `tests/mod.rs`. Each of the three removed
      definitions was replaced by a `//` note saying where it went, so no doc
      comment was left dangling above an unrelated item.

Acceptance: MET, and the stronger half of it exactly.
`rustup run 1.96.0 cargo test -p mfb_wire --no-fail-fast` → **14 passed**, all
reported as `bytes::tests::*` under `mfb_wire`.
`rustup run 1.96.0 cargo test --bin mfb --no-fail-fast binary_repr` →
**171 passed; 0 failed**.
`git diff --stat HEAD -- src/binary_repr/{reader,writer,sections,builder}.rs`
prints **nothing**: zero changes to all four. The glob re-export reached every
one of the ~200 call sites (`cursor_u32` alone has 128) with no call-site edit.
Commit: baf71a193 (with Phase 1)

### Phase 3 — Move the MFP framing (largest blast radius)

- [x] Create `wire/src/mfp.rs` holding `MFP_MAGIC`, `FIXED_PREFIX_LEN`, and the
      fixed-prefix field readers, taken from the manifest reader (the copy that
      already enforces per-field byte limits and UTF-8, since it is the
      stricter and its extra guards are parameters, not policy). The module doc
      carries the **three**-decoder policy table (see Corrections in this file)
      and the field-order diagram.
- [x] Move the signature-header rule there. Both copies were read and **diffed
      before merging**: `diff` of the two function bodies reports no
      difference — they agree exactly, so neither was chosen over the other and
      there is no divergence finding to record. `src/binary_repr/mod.rs`
      re-exports it under the old name so `validate_mfp_signature_header`
      call sites and the existing `reader_tests` test are unchanged.
- [x] Re-point `read_mfp_header` at the shared readers, **keeping** its `ident`
      `required=false` argument and its `validate_package_name(&name)` call.
      Deleted its five private helpers.
- [x] Re-point `parse_mfp_package` at the shared readers, **keeping** its
      `ident` `required=true` argument and its absence of a name charset guard.
      Deleted the unnamed literal `MFP_MAGIC`, the private `FIXED_PREFIX_LEN`,
      `validate_signature_header`, and the private
      `read_mfp_string`/`read_mfp_bytes`/`read_u16`/`read_u32`/`read_u64`
      (**six** helpers plus two constants, not the four the plan listed).
- [x] Update the bug-340 B8 note at the top of `src/binary_repr/mod.rs` in
      place — kept, not deleted. It now names `mfb_wire::mfp` as the shared
      layer, enumerates all **three** decoders with the specific guard each
      one keeps, and says why the guards are call-site arguments.
- [x] Tests: **9** in `wire/src/mfp.rs` — the frozen magic/prefix values,
      round-trip and offset advance, truncated field vs truncated header,
      over-cap reported as a limit rather than as truncation, invalid UTF-8,
      the `required` divergence, the signature-header rule's five outcomes,
      the header scalar readers' error text, and a PKG-07 wrap-offset negative.
      Plus **two** cross-crate tests in `src/manifest/package.rs`:
      `the_two_mfp_decoders_still_disagree_about_an_empty_ident_by_design` and
      `only_the_manifest_reader_applies_the_name_charset_guard`.
- [x] Added task: fix the spec provenance citation that the move invalidated.
      `src/docs/spec/package/01_container-format.md` cited
      `[[src/binary_repr/reader.rs:validate_mfp_signature_header]]`; the symbol
      left that file. `spec_citations_resolve` is **file-level only**
      (`.ai/testing-gates.md`), so it would have stayed green with the citation
      silently decayed. Re-pointed at `[[wire/src/mfp.rs:validate_signature_header]]`
      and the gate re-run.

Acceptance: MET.
`cargo test -p mfb_wire` → 23 passed (14 `bytes` + 9 `mfp`).
`cargo test --bin mfb manifest::package` → 81 passed, 0 failed, including both
cross-crate divergence tests.
`cargo test -p mfb_repository --lib` → 381 passed, 0 failed.
`cargo test --bin mfb citations_resolve` → `spec_citations_resolve` ok.
`scripts/artifact-gate.sh target/release/mfb all` → 1427 tests, 1593 builds,
**2001 goldens checked, 0 diffs**, with `git status tests/` clean (no golden
regenerated). A rebuilt `packages/jwt/jwt.mfp` is `cmp`-identical to the
pre-Phase-3 build.
Runtime proof that the moved framing still decodes *existing* bytes: both
`mfb pkg info` and `mfb pkg doc` succeed against a `jwt.mfp` written by the
pre-change compiler (the doc run produced a 54,788-byte page).
The whole-workspace `cargo test --no-fail-fast` — the run that covers the 160
committed `.mfp` fixtures' `rt-behavior` tests — is the plan-wide final gate in
§5 rather than a per-phase one.
The cross-crate divergence tests are what prove bug-340 B8 was honored rather
than silently undone; both were **red first**, for a fixture reason recorded in
Corrections.
Commit: c2ef2dc23

## Validation Plan

- **Tests:** `wire/src/bytes.rs` (**14** moved — see Corrections),
  `wire/src/mfp.rs` (9 new field-reader and signature-header tests, including
  truncation, over-cap, invalid-UTF-8 and wrap-offset negatives), the **two**
  cross-crate divergence tests in `src/manifest/package.rs`, and the 3
  package-meta tests relocated to `src/binary_repr/tests/reader_tests.rs`.
- **Coverage check:** DONE. `mfb_wire` is in `default-members`, so its tests are
  inside a bare `cargo test` — the hole bug-347 fixed for `repository/`.
  Confirmed by `cargo metadata --no-deps` → 3 members and by the crate's own
  `Running unittests src/lib.rs (target/debug/deps/mfb_wire-…)` line reporting
  23 passed.
- **Runtime proof:** DONE. `mfb pkg info` and `mfb pkg doc` both succeed against
  a `jwt.mfp` written by the **pre-change** compiler (saved before Phase 2 as
  `/tmp/p126-bytecheck/jwt-prechange.mfp`, 702,134 B); the doc run wrote a
  54,788-byte page. Note the plan's literal path does not work as written:
  `packages/*/*.mfp` are gitignored build artifacts and absent from a fresh
  worktree, so the package must be built first (`mfb build packages/jwt`).
- **Byte-identity:** `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.
  Per `.ai/testing-gates.md`, run it against the **release** binary. Acquire
  `scripts/gate-lock.sh` first — `artifact-gate.sh` and `test-accept.sh` are
  mutually exclusive in one worktree and refuse with exit 98.
- **`.mfp` byte-identity:** rebuild `packages/jwt/jwt.mfp` and confirm it is
  byte-identical to the pre-change build (`cmp`). The `mfp` dump kind is
  deliberately *not* in the artifact gate (`.ai/testing-gates.md`), so this check
  must be run by hand.
- **Doc sync:** DONE — **but this line originally under-counted.** It claimed one
  item more than the plan listed; plan-126-D's citation sweep later found **two
  more** spec citations this sub-plan broke (`MFPC_MAJOR_VERSION` in
  `08_artifacts.md` and `02_binary-representation.md`), both since re-pointed —
  see Corrections. The record as first written:
  `.ai/build-tooling.md` (stale workspace claim *and* the stale second-fmt-pass
  claim beside it, Phase 1); the bug-340 B8 note in `src/binary_repr/mod.rs`
  (Phase 3). The plan asserted "no `src/docs/spec/**` change — this is internal
  crate layout", which is **wrong**: `src/docs/spec/package/01_container-format.md`
  carries a `[[path:Symbol]]` provenance citation for the very function Phase 3
  moves. Corrected, and recorded in Corrections along with why the citation gate
  could not have caught it.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`; `docker build -f
  repository/Dockerfile .`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run
  1.96.0 cargo fmt)` — then confirm the new crate was reached, since AGENTS.md's
  two-pass command predates the workspace table.

## Open Decisions

- **Crate name and directory.** **RESOLVED: `mfb_wire` in `wire/`**, taking the
  recommendation. The contents really are wire formats specifically, and the
  name matches the `repository/` → `mfb_repository` convention. (§3)
- **Does `Section` belong in `bytes.rs` or its own `mfpc.rs`?** **RESOLVED for
  now: `bytes.rs`, and plan-126-C should move it.** Landed in `bytes.rs` as
  recommended — but the decision is now better informed than when it was
  written. `MFPC_MAJOR_VERSION` had to come along too (Corrections), so
  `bytes.rs` currently holds `Section`, `encode_sections` *and* the container
  major version: that is an `mfpc` module in all but name, sitting inside a file
  whose stated job is arch-neutral byte primitives. When plan-126-C creates
  `wire/src/mfpc.rs` for the section ids and the section-table reader, these
  three should move into it, leaving `bytes.rs` genuinely primitive-only. Each
  carries a doc note saying so. (§Phase 2)

## Corrections

- **`MFPC_MAJOR_VERSION` had to move with `Section` and `ABI_HASH_LEN`, and the
  plan did not list it.** `encode_sections` stamps it into every container it
  frames (`grep -n "MFPC_MAJOR_VERSION" wire/src/bytes.rs`), so moving
  `encode_sections` without it does not compile. It sits in `bytes.rs` with a
  note that plan-126-C relocates it to the `mfpc` module alongside the section
  ids and the section-table *reader*. This also partly answers B's second Open
  Decision in the affirmative: `Section` and `encode_sections` will want to
  follow it into `mfpc.rs` in C.

- **Only 14 of `util_tests.rs`'s 17 tests belonged in `mfb_wire`.** The file's own
  banner said it covered "util.rs — low-level cursor readers, capacity guards,
  section framing", but three of its tests —
  `package_meta_section_round_trips_and_is_omitted_when_empty`,
  `an_unknown_package_meta_field_id_is_skipped_not_rejected` and
  `an_over_cap_description_is_rejected_at_read_time` — exercise
  `encode_package_meta` / `read_package_meta`, the MFPC **section-18** codec that
  lives in `reader.rs` and is staying in the compiler. Moving them made
  `mfb_wire` fail to compile (`cannot find function encode_package_meta`,
  `cannot find manifest in crate` — the third reaches
  `crate::manifest::MAX_DESCRIPTION_BYTES`), which is how they were caught. They
  now live in `src/binary_repr/tests/reader_tests.rs` (32 → 35 tests) under a
  banner recording that they were misfiled. So the split is 14 moved / 3
  relocated within the compiler, not 17 moved.

- **The `.ai/build-tooling.md` correction is bigger than the plan described.**
  The plan said to fix the "there is no `[workspace]` table" claim and to
  restate that `repository/` keeps its own `cargo fmt` pass. The second half is
  no longer true either: with `repository/` a workspace *member* (bug-347),
  `cargo fmt --all` reaches it. Measured rather than assumed — appended
  `pub fn probe_fmt(   )->u8{1}` to both `repository/src/validation.rs` and
  `wire/src/lib.rs`, ran `cargo fmt --all`, and **both** were reformatted; the
  probes were then removed and `git diff --stat` confirmed clean. The doc now
  says the second pass is redundant-but-harmless, and warns that the real
  present-day trap is the opposite one: `--all` reformats *other sessions'*
  files in a shared checkout, so `git diff --stat` afterwards is mandatory.
  AGENTS.md's two-pass command is left alone — it is still correct, just no
  longer necessary.

- **There are three `.mfp` fixed-prefix decoders, not two.** § Current State
  tabulates the manifest reader and the registry reader. A third exists:
  `mfp_binary_repr_payload` (`grep -n "fn mfp_binary_repr_payload"
  src/binary_repr/reader.rs`), which decodes the same prefix with *no* per-field
  byte limits, using `read_length_prefixed` / `skip_length_prefixed` — already
  shared primitives, so Phase 2 covered it for free. It is a third policy, not a
  third copy, and the bug-340 B8 note's "the two full decoders" wording
  undercounts. Phase 3 must not fold it in either.

- **The two signature-header rules agreed exactly — no divergence to resolve.**
  Phase 3 required reading both first and recording a finding if they differed.
  Diffing the two function bodies (`diff <(sed -n '/^pub(crate) fn
  validate_mfp_signature_header/,/^}/p' src/binary_repr/reader.rs | tail -n +5)
  <(sed -n '/^fn validate_signature_header/,/^}/p' repository/src/package.rs |
  tail -n +2)`) reports **no difference**. Only the signature formatting
  differed (one wrapped its parameters, one did not). So the merged function is
  that rule, not a choice between two.

- **The move invalidated a spec provenance citation, and the gate that exists
  for those cannot see it.** `src/docs/spec/package/01_container-format.md`
  cited `[[src/binary_repr/reader.rs:validate_mfp_signature_header]]`.
  `spec_citations_resolve` is **file-level only** — `.ai/testing-gates.md` says
  so explicitly: "it passes as long as the cited file exists, even if the symbol
  moved out of it". `reader.rs` still exists, so the gate would have stayed
  green with the citation pointing at a symbol no longer there. Found by
  grepping for the old symbol name rather than by any test. Re-pointed to
  `[[wire/src/mfp.rs:validate_signature_header]]`. **Any future sub-plan that
  moves a cited symbol must grep; the gate will not tell it.**

- **Both cross-crate divergence tests were red on first run, from the fixture
  and not the code.** The existing `build_mfp` helper writes `proofSig` and
  `attestationSig` as 64 zero bytes, and `parse_mfp_package` additionally
  enforces that an *unsigned* package carries none of the trust chain — so the
  registry rejected the fixture with "unsigned .mfp package must not carry
  proofSig", which is not the `ident` guard under test. A fixture rejected for
  the wrong reason would have made both tests pass vacuously the moment the
  assertion was loosened to `is_err()`. Added
  `build_mfp_for_both_decoders`, whose chain fields are genuinely empty, so both
  decoders consider the fixture well-formed apart from the one field being
  tested. The helper's doc comment records why it cannot just reuse `build_mfp`.

- **The registry had six private helpers and two constants to delete, not
  four helpers.** Phase 3 listed `read_mfp_string`/`read_mfp_bytes`/`read_u16`/
  `read_u32` plus the literal magic. Also present and now removed:
  `read_u64`, `validate_signature_header`, and a private `FIXED_PREFIX_LEN`.

- **This sub-plan's doc-sync record was incomplete: 2 spec citations it broke
  went unnoticed until plan-126-D.** § Validation Plan reported doc sync DONE and
  counted one re-pointed citation (`validate_mfp_signature_header`). But Phase 2
  moved `MFPC_MAJOR_VERSION` out of `src/binary_repr/mod.rs`, replacing the local
  `const` with a glob re-export. That broke
  `[[src/binary_repr/mod.rs:MFPC_MAJOR_VERSION]]` in
  `src/docs/spec/architecture/08_artifacts.md` and
  `src/docs/spec/package/02_binary-representation.md`, and the file-level
  `spec_citations_resolve` cannot see it. Found by a mechanical sweep in
  plan-126-D that checks every `[[path:Symbol]]` for a *definition* in the cited
  file, diffed against the fork commit `e66e594a4` to separate plan-126's breakage
  from 353 pre-existing flags. Both citations now read
  `[[wire/src/mfpc.rs:MFPC_MAJOR_VERSION]]`, the constant's final home after
  plan-126-C. **Durable lesson:** grepping `src/docs/spec/` for the moved
  *function's* name is not enough — constants moved alongside it decay too, so
  sweep for every symbol that left the file.

- **Populations re-measured 2026-09-12** (plan figures in parentheses): committed
  `.mfp` fixtures **160** (159); `util.rs` **29** `pub(super) fn` converted — the
  plan's 28 counted `^pub(super) fn` and missed the one inside `impl Section`.
  `abi.rs` is now **1,488 lines / 29 tests** (1,063 / 21), which matters for
  plan-126-C rather than here.

## Summary

The engineering risk is concentrated in Phase 3, where two independently-written
decoders start sharing an offset-advancing reader; the 159 committed `.mfp` fixtures
and the byte-identity gate are what make that safe. Phases 1 and 2 are cheap and
mechanical — the glob-import structure means 200+ call sites move for free. What is
deliberately left untouched: `crypto`, `MfpPackage`, the `server` DTOs, `client` and
`local`, and both decoders' guard policies.
