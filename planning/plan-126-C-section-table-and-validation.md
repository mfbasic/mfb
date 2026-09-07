# plan-126-C: One MFPC section table, one validation module, one sanitizer

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-B

Ports `repository/src/abi.rs` onto the shared MFPC framing and moves the last three
duplicated things — the section-id/wire-enum constants, the package-name validators,
and the terminal sanitizer — into `mfb_wire`. This is the phase where the *tamper
checks on a signed payload* stop existing in two independently-written copies.

Behavioral outcome: the registry's MFPC section-table reader gains the two guards
the compiler's has and it lacks — an MFPC major-version check and a checked
`u64 → usize` conversion — and every constant the registry currently restates from
the compiler is imported instead.

References:

- `repository/src/abi.rs:1-8` — "this crate does not depend on the compiler crate",
  the comment that motivated every restatement this sub-plan removes.
- `src/binary_repr/reader.rs:383-427` — `read_binary_repr_package`, which contains
  the compiler's section-table decode inline.
- `src/terminal_safe.rs:1-11` — the bug-489 re-export shim and its own statement of
  the problem.
- AGENTS.md § "Never edit a test/golden to pass" — Phase 2 changes what the registry
  accepts; the four-question gate applies to any test that pushes back.

## Prerequisites

See plan-126-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-126-B complete (`mfb_wire` exists with `bytes` + `mfp`) | `ls wire/src/bytes.rs wire/src/mfp.rs` → both exist | NOT MET |

If plan-126-B is not complete, this sub-plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- One `read_section_table` in `mfb_wire`, used by both `src/binary_repr/reader.rs`
  and `repository/src/abi.rs`, enforcing the union of today's guards.
- MFPC section ids, the `libc`/`libtype` wire encodings, `NATIVE_LIBRARY_HASH_LEN`,
  `ABI_HASH_LEN`, `ABI_FORMAT_VERSION` and `MAX_DESCRIPTION_BYTES` declared once.
- One `validation` module in `mfb_wire` holding both package-name policies under
  names that say what each guards, plus `fold_owner`, `validate_owner_name`,
  `validate_version`, `validate_ident` and the three limits.
- `terminal_safe` implemented in `mfb_wire`; both crates import it and the bug-489
  shim direction is reversed.

### Non-goals (explicit constraints)

- **No change to the MFPC wire format**, section ids, or field layout.
- **The two package-name policies keep their current accept sets** unless the Open
  Decision below is resolved to unify them — and if it is, that is a deliberate,
  tested behavior change, never a side effect of the move.
- **`crypto`, `MfpPackage`, the `server` DTOs, `client` and `local` still do not
  move** (plan-126-B § Non-goals).
- The registry's best-effort posture toward optional sections is preserved: a
  package with no section 10 or 18 is still normal, not an error.

## 2. Current State

### The section table is decoded twice, with different guards

| Guard | compiler (`src/binary_repr/reader.rs:383-427`) | registry (`repository/src/abi.rs:317-343`) |
|---|---|---|
| MFPC magic | yes (`:384`) | yes (`:318`) |
| **MFPC major version == 2** | **yes** (`:389-395`, `MFPC_MAJOR_VERSION`, `src/binary_repr/mod.rs:82`) | **no** |
| section-table length overflow | yes, `checked_add`/`checked_mul` | yes, same shape |
| truncated section table | yes | yes |
| truncated section | yes | yes |
| duplicate section id (PKG-06) | yes (`:420-427`) | yes (`:339-341`) |
| **`u64 → usize` conversion** | **`checked_usize`** (`:412-413`) | **`as usize`** (`:334-335`) |

The registry's comment at `:339` — "matches the compiler reader" — is the tell: two
copies kept in sync by hand, and they have already drifted on two rows.

The compiler's decode is **not** a separate function; it is inline inside
`read_binary_repr_package` (`src/binary_repr/reader.rs:383`). Extracting it is part
of this work.

### Constants the registry restates

`repository/src/abi.rs:11-23` declares `MFPC_MAGIC`, `SECTION_MANIFEST`,
`SECTION_STRING_POOL`, `SECTION_NATIVE_LIBRARY_TABLE`, `SECTION_ABI_INDEX`,
`SECTION_PACKAGE_META`, `PACKAGE_META_FIELD_DESCRIPTION`, `MAX_DESCRIPTION_BYTES`,
`ABI_FORMAT_VERSION`, `ABI_HASH_LEN`; `:28-37` declares `WIRE_LIB_TYPE_VENDOR`,
`NATIVE_LIBRARY_HASH_LEN` and `WIRE_LIBC_UNSPECIFIED`/`GLIBC`/`MUSL`, with a comment
citing `src/binary_repr/mod.rs:353-359` as the source of truth. Compiler-side homes
are `src/binary_repr/mod.rs:39-74` (ids) and `:82`, `:100`.

### The two package-name validators are closer than they look

Read both in full:

- `src/manifest/package.rs:58-70` — first char `[A-Za-z0-9_]`, remaining
  `[A-Za-z0-9_.-]`. No length cap, no distinct empty-input message (an empty name
  fails because `chars.next()` is `None`).
- `repository/src/validation.rs:47-72` — explicit empty check, `len > 128` cap
  (`PACKAGE_LIMIT`), first char `[A-Za-z0-9_]`, all chars `[A-Za-z0-9_.-]`.

**The charsets are identical.** The only substantive difference is the registry's
128-byte cap, plus different error strings. This is a smaller divergence than the
existence of two functions implies.

### `terminal_safe` is already shared, in the wrong direction

`src/terminal_safe.rs` is a 12-line `pub(crate) use mfb_repository::terminal_safe::{is_terminal_unsafe, safe};`
whose doc comment says the implementation lives in the registry crate "because
`mfb_repository` cannot depend on `mfb`" (bug-489). The 100-line implementation is
in `repository/src/terminal_safe.rs`. A terminal sanitizer is not registry code.

### Measured populations

| What | Count | Command |
|---|---|---|
| `repository/src/abi.rs` lines / tests | 1,063 / 21 | `wc -l repository/src/abi.rs`; `grep -c '#\[test\]' repository/src/abi.rs` |
| `repository/src/validation.rs` lines / tests | 172 / 6 | same form |
| `repository/src/terminal_safe.rs` lines / tests | 100 / 4 | same form |
| `terminal_safe::{safe,is_terminal_unsafe}` call sites — compiler / registry | 37 / 2 | `grep -rho 'terminal_safe::\(safe\|is_terminal_unsafe\)' src --include='*.rs' \| wc -l`; same over `repository/src` |
| `validation::*` call sites in the registry | 12 | `grep -rho 'validation::[a-z_]*' repository/src --include='*.rs' \| wc -l` |
| `validate_package_name` references in `src/` | 16 | `grep -rho validate_package_name src --include='*.rs' \| wc -l` |
| Constants restated in `abi.rs` | 15 | `sed -n '11,37p' repository/src/abi.rs \| grep -c '^const'` |

### Verified properties

- **The registry really is missing the major-version check.**
  `grep -n "MFPC_MAJOR\|major" repository/src/abi.rs` returns one hit, at `:486`,
  inside a *test* fixture builder (`put_u16(&mut bytes, 2); // major`) — never a
  validation. The compiler's check is at `src/binary_repr/reader.rs:389-395`.
- **A stricter reader will not reject a publish; it will silently empty a field.**
  Read `abi_index_json` (`repository/src/abi.rs:306-315`): every parse error maps to
  `serde_json::json!({})`. `parse_package_description` and `parse_vendor_blobs`
  likewise return `Ok(None)`/`Ok(vec![])` for a non-MFPC payload
  (`:214-217`, `:108-118`). So a newly-strict `read_section_table` changes *recorded
  metadata*, not accept/reject — which is more insidious, and is why Phase 2 needs
  an explicit test rather than "the publish tests still pass".
- **The charsets of the two name validators are identical**, established by reading
  both functions in full (see above), not by their names or call sites.

## 3. Design Overview

Three independent moves, ordered by blast radius:

1. **`wire/src/validation.rs`** — both name policies plus the owner/version/ident
   validators and limits. Pure functions, no callers change semantics.
2. **`wire/src/terminal_safe.rs`** — the implementation moves out of the registry;
   `repository/src/terminal_safe.rs` becomes the re-export shim, and
   `src/terminal_safe.rs`'s shim re-points. 39 call sites across both crates are
   untouched because both shims keep the same paths.
3. **`wire/src/mfpc.rs`** — `MFPC_MAGIC`, `MFPC_MAJOR_VERSION`, the section ids, the
   wire enums, the limits, and `read_section_table`. Both crates re-point.

**Where correctness risk concentrates:** move 3. It changes tamper checks on bytes
covered by a package signature, and the failure mode is silent (an emptied ABI index
rather than a rejected publish). It lands last, with a test that asserts the *new*
guard actually fires.

**Where design uncertainty concentrates:** whether unifying the two name validators
is safe (Open Decision). Do not resolve it by guessing; measure whether any package
name over 128 bytes exists in the tree first — that is a one-line command and it is
Phase 1's first task.

**Byte-identity is this sub-plan's gate for moves 1 and 2** (pure code motion; no
codegen effect expected, and `scripts/artifact-gate.sh` should report `diffs=0`).
For move 3 it is the wrong gate — the registry's behavior legitimately changes — so
the check there is the new negative tests plus the repository crate's 351 lib tests.
A `diffs=0` from the artifact gate proves nothing about move 3 and must not be cited
for it.

**Rejected alternative — leave `abi.rs` alone and restate section id 17 for the doc
table.** That is the fourth restatement of the same knowledge in a file that already
carries a comment apologizing for the first three, and it leaves the drifted
tamper-check rows drifted.

**Rejected alternative — make the compiler adopt the registry's laxer reader.** The
divergences both favor the compiler (a major-version check and a checked cast are
strictly better); the union is the compiler's set.

## Compatibility / Format Impact

- **Registry behavior changes** for a malformed or wrong-version MFPC payload: what
  previously produced an empty `abiIndex`/`description` via a partially-successful
  parse now produces an empty one via an explicit early error. Externally the JSON
  shape is identical; the difference is which packages land with empty metadata.
  A payload the *compiler* would refuse to load is the only input affected.
- No change to the `.mfp`/MFPC format, section ids, HTTP routes, or DB schema.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Validation module

- [ ] Measure first: `git ls-files '*.mfp' | while read f; do mfb pkg info "$f"; done`
      (or read the header name of each of the 159 committed fixtures) and record
      the longest package name in the tree. This decides the Open Decision below.
- [ ] Create `wire/src/validation.rs` holding, from `repository/src/validation.rs`:
      `OWNER_LIMIT`, `PACKAGE_LIMIT`, `VERSION_LIMIT`, `fold_owner`,
      `validate_owner_name`, `validate_version`, `validate_ident`; and both name
      policies renamed to say what they guard —
      `validate_path_component_name` (from `src/manifest/package.rs:58`, guards
      `packages/<name>.mfp`) and `validate_registry_package_name` (from
      `repository/src/validation.rs:47`, guards the log payload and `/index/<ident>`
      route). Keep each function's current error strings verbatim.
- [ ] Re-point `repository/src/validation.rs` to a re-export shim, and
      `src/manifest/package.rs:58` likewise, so the 12 registry and 16 compiler
      call sites are unchanged.
- [ ] Move the 6 tests from `repository/src/validation.rs` into
      `wire/src/validation.rs` and add one that documents the difference explicitly:
      a 200-character all-legal-charset name **passes** `validate_path_component_name`
      and **fails** `validate_registry_package_name`, with a comment naming
      `PACKAGE_LIMIT` as the only substantive divergence.

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes; the divergence test
above is present and green, so the next reader learns the two policies differ only
by a length cap rather than having to diff them.
Commit: —

### Phase 2 — Terminal sanitizer

- [ ] Move `repository/src/terminal_safe.rs`'s 100-line implementation and its 4
      tests to `wire/src/terminal_safe.rs`.
- [ ] Replace `repository/src/terminal_safe.rs` with a re-export shim, and update
      `src/terminal_safe.rs`'s shim to point at `mfb_wire`. Rewrite both doc
      comments: the bug-489 rationale ("`mfb_repository` cannot depend on `mfb`") is
      now obsolete and would mislead — say instead that the sanitizer is shared
      wire-adjacent code with one home in `mfb_wire`.
- [ ] Confirm neither shim removal orphaned a doc comment onto a neighbouring item.

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes with the 4 tests
reported under `mfb_wire`; `git diff --stat` shows **no** change to any of the 39
call sites (37 compiler + 2 registry) — only the two shim files and the moved
implementation.
Commit: —

### Phase 3 — One MFPC section table (largest blast radius)

- [ ] Create `wire/src/mfpc.rs` with `MFPC_MAGIC`, `MFPC_MAJOR_VERSION`, every
      `SECTION_*` id from `src/binary_repr/mod.rs:39-74`, `PACKAGE_META_FIELD_DESCRIPTION`,
      `ABI_FORMAT_VERSION`, `ABI_HASH_LEN`, `MAX_DESCRIPTION_BYTES`,
      `NATIVE_LIBRARY_HASH_LEN`, and the `WIRE_LIBC_*` / `WIRE_LIB_TYPE_*` encodings.
      Each keeps its existing doc comment; the "restated here" apologies in
      `repository/src/abi.rs:11-37` are deleted, not moved.
- [ ] Extract the section-table decode from `read_binary_repr_package`
      (`src/binary_repr/reader.rs:396-427`) into
      `mfb_wire::mfpc::read_section_table`, keeping **all** compiler guards: magic,
      `MFPC_MAJOR_VERSION`, `checked_add`/`checked_mul` on the table length,
      truncation, `checked_usize` on offset and length, and the PKG-06 duplicate-id
      rejection with its explanatory comment.
- [ ] Re-point `read_binary_repr_package` at it. The returned map type differs
      (`HashMap` vs the registry's `BTreeMap`) — pick one in the shared signature
      and adapt the other caller; prefer `BTreeMap` so section iteration order is
      deterministic.
- [ ] Delete `repository/src/abi.rs:317-343 read_section_table` and re-point its
      five callers (`parse_vendor_blobs:107`, `parse_package_description:213`,
      `parse_manifest_metadata:266`, `parse_abi_index:293`, and the fifth found by
      `grep -n read_section_table repository/src/abi.rs`).
- [ ] Preserve the best-effort posture at each registry call site: a payload that is
      not an MFPC container must still yield "no such section", not an error, exactly
      as `parse_vendor_blobs:108-118` and `parse_package_description:214-217` do today.
- [ ] Tests in `wire/src/mfpc.rs`: duplicate section id rejected; truncated table
      rejected; truncated section rejected; **a container declaring MFPC major
      version 3 rejected** (the guard the registry lacked); a `u64` offset that does
      not fit `usize` rejected.
- [ ] Test in `repository/src/abi.rs`: a payload with MFPC major version 3 now
      yields empty metadata *through the explicit error path*, asserted directly —
      not inferred from a green publish test, because `abi_index_json` swallows
      errors to `{}` (`repository/src/abi.rs:306-315`).

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes; the five new
`mfb_wire` negative tests are green; and the registry test proves the major-version
guard now fires there. `scripts/artifact-gate.sh target/release/mfb all` reports
`diffs=0` — the compiler's decode is unchanged in behavior, only relocated.
Commit: —

## Validation Plan

- **Tests:** `wire/src/validation.rs` (6 moved + 1 divergence),
  `wire/src/terminal_safe.rs` (4 moved), `wire/src/mfpc.rs` (5 new negatives),
  `repository/src/abi.rs` (21 existing must stay green + 1 new major-version test).
- **Coverage check:** the 21 `abi.rs` tests are the real regression net for Phase 3;
  confirm they run (`cargo test -p mfb_repository abi:: 2>&1 | tail`) rather than
  assuming a green workspace run exercised them.
- **Runtime proof:** publish a package to a local `mfb-repo` and confirm
  `GET /packages/<ident>` still reports a non-empty `abiIndex` and the correct
  `description` — this is what proves the stricter reader did not silently empty
  metadata for *valid* packages, which is Phase 3's specific failure mode.
- **Byte-identity:** `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.
  Acquire `scripts/gate-lock.sh` first (exit 98 means a rival script holds it).
  Valid evidence for Phases 1–2 and for the compiler half of Phase 3; **not**
  evidence for the registry half.
- **Doc sync:** delete the now-false "restated here because this crate does not
  depend on the compiler crate" comments (`repository/src/abi.rs:1-8`, `:11-37`) and
  the bug-489 rationale in both `terminal_safe` shims. `.ai/` topic docs: check
  `grep -rn "abi.rs\|terminal_safe" .ai/` for anything that describes the old layout.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`; `tests/cli_repo_publish.rs`;
  `docker build -f repository/Dockerfile .`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Unify the two package-name validators into one?** Their charsets are identical;
  the registry's adds a 128-byte cap. Recommended: **keep both, renamed** (as in
  Phase 1) until the Phase 1 measurement shows no name in the tree exceeds 128
  bytes — then unifying becomes a safe, separately-tested follow-up rather than a
  side effect of a code move. Alternative: unify now and accept that
  `read_mfp_header` starts rejecting names over 128 bytes. (§Phase 1)
- **`HashMap` or `BTreeMap` for the shared section table?** Recommended `BTreeMap`
  (the registry's choice) so iteration order is deterministic — `HashMap` deciding
  an order has bitten this tree before. Confirm no compiler caller depends on
  `HashMap`-specific API. (§Phase 3)

## Corrections

<!-- Fill in during execution. Expected candidates: the longest committed package
     name (Phase 1 measurement), and the fifth `read_section_table` caller if the
     grep finds a different count than four. -->

## Summary

The engineering content is Phase 3: two hand-synchronized copies of a tamper check
on signed bytes become one, and in the process the registry gains an MFPC
major-version check and a checked width conversion it has been missing. The
insidious part is that the failure mode is silent — `abi_index_json` maps every
error to `{}` — so the phase's acceptance deliberately asserts the guard fires
rather than trusting a green publish test. Phases 1 and 2 are mechanical; the
package-name unification is explicitly deferred behind a measurement rather than
folded into a code move.
