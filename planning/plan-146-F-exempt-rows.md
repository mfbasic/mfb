# plan-146-F: The `Exempt` rows, each proven copy-free

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-E

Prerequisites: see plan-146-A.

Fifteen `String` self-update rows get `Exempt` instead of an arm. Each needs a
proof, citing its lowering, that `s` is only read and never copied:

- **5 not-derived rows** (findings §3.4): the result is not a function of `s`'s
  bytes. `fs::readText`, `fs::canonicalPath`, `io::input`, `os::getEnv`,
  `os::getEnvOr`.
- **10 MFBASIC-body rewrites** (plan-146-A Open Decision 2): the output is a new
  byte stream the helper builds by reading `value`. `encoding::formUrlDecode
  formUrlEncode htmlEscape htmlUnescape percentDecode percentEncode punycodeDecode
  punycodeEncode`, `net::percentDecode`, `regex::replace`.

plan-142-E set the precedent: `Exempt` is earned, not declared. Two of its rows
copied `x` (`argon2id`, `shake256`) and were fixed before they were exempted.
Here, finding F4 names four helpers that copy `s`, and one helper body is known to
copy `value` (below). Each is fixed first.

## 1. Goal

- The 15 rows flip `Pending("F")` → `Exempt { reason, proof }`. Each `proof` cites
  the `file:symbol` that shows `s` is only read. Their `cases.tsv` lines flip
  `pending:F` → `exempt`: the harness's peak-live-bytes bound, plus the value check.
- F4 is fixed: `os::getEnv`, `os::getEnvOr`, `fs::readText` and
  `fs::canonicalPath` pass the host call a pointer to the `String`'s own bytes,
  which already end in a NUL (`03_heap-values.md`, "Standalone String"), instead
  of an arena copy.
- Every MFBASIC body that copies `value` either stops copying or gets an arm
  (Phase 1 decides which, per row).

### Non-goals

- No change to any result or error. A `String` with an interior NUL reaches the
  host truncated at that NUL, both before and after the F4 fix: the copy loop
  copies the NUL too.
- No native re-implementation of any codec.

## 2. Current State

- **F4.** `marshal_cstring` (`os/gen_shared.rs:130`) allocates `len + 1` arena
  bytes, copies the bytes and appends a NUL. The helper frees it at `done`
  (bug-574). It is called by `gen_env.rs:158` (`getEnv`/`getEnvOr`),
  `func_set_env.rs:57,67`, `func_unset_env.rs:46` and `func_has_env.rs:47`.
  `fs::readText` copies its path in `{symbol}_path_copy_loop`
  (`fs/gen_atomic_write.rs:811`, `:1091`). `fs::canonicalPath` marshals `c_path`
  (`fs/gen_canonical.rs`). `io::input` only writes the prompt to stdout
  (`gen_read_line_family.rs`, findings r19).
- **A known body copy.** `__encoding_htmlEscape`
  (`encoding/func_html_escape.rs:40-48`) starts `MUT out AS String = text`. Binding
  a parameter to a `MUT` copies it (`value_needs_owning_copy`, findings B.3 fact 3).
  Then it runs five `out = strings::replace(out, …)`, which are in place after
  letter E. So `s = encoding::htmlEscape(s)` copies `s` once and rewrites the copy.
  The other nine bodies are unread (Phase 1).
- The harness's `exempt` status (plan-142-E) asserts a peak-live-bytes bound and
  the value check.

## 3. Design

**F4.** Replace each copy with the address `block + 8`. The bytes are
NUL-terminated and the block outlives the host call: it is the caller's argument,
borrowed for the call (findings B.3 fact 4). The scratch-release bookkeeping those
copies carried (`HelperScratch`, `emit_helper_scratch_release`) goes with them.
Open Decision 1 decides whether `marshal_cstring`'s other callers change too.

**A body that copies `value`.** Phase 1 reads the 10 bodies. For each copy it
finds, there are two options:

- **stream:** rewrite the body so it reads `value` and appends to an output built
  from empty. This is plan-142-E's fix shape.
- **arm:** when the body is a chain of in-place self-updates on a copy of its
  argument, as `htmlEscape` is, the self-update `s = f(s)` can run that chain on
  `s` directly. That is an arm, `ArmId::StrChain`, which expands the body's
  statements onto the binding.

Recommended: stream, unless streaming changes the result or its error behavior.
Record the choice per row here. An `arm` choice moves the row out of the Exempt
set and into this letter's arm list. Record it as a Correction, with the
recounted row totals.

Risk: low for results. F4 removes a copy the host never needed. The body rewrites
are MFBASIC, so the existing `encoding`, `net` and `regex` rt-behavior fixtures
guard their results.

Rejected alternatives:

- **Exempt without reading the bodies.** A body copy is exactly what plan-142-E
  found in two of its rows, and `htmlEscape` already shows one here.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Read the 15 lowerings

- [ ] For each of the 10 MFBASIC bodies: every statement that copies `value`
      (a `MUT`/`LET` bound to it, a `mid`/`left` over the whole of it, a
      concatenation seeded with it), with a line citation. Record the stream-or-arm
      choice per copying row.
- [ ] For the 5 not-derived rows: confirm F4's four copies are the only copies of
      `s`, and that `io::input` makes none.
- [ ] Measure: the rt-behavior fixtures that exercise each of the 15 rows
      (`grep -rl '<pkg>::<f>(' tests/rt-behavior | wc -l` per row). A row with 0
      fixtures gets one in Phase 3.

Acceptance: recorded here with citations and the per-row fixture counts
(est. 40 min).
Commit: —

### Phase 2: F4

- [ ] `getEnv`/`getEnvOr`, `readText` and `canonicalPath` pass `block + 8` to the
      host. Remove the copies and their scratch release.
- [ ] Runtime: the existing `os` and `fs` fixtures for these four pass. Add a
      case per row whose argument is a `MUT` binding self-updated in a loop
      (`s = os::getEnvOr(s, "x")`).

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/os'`
and `… 'rt-behavior/fs'` → pass (est. 5 min).
  Expected golden diffs: every fixture that calls one of the four (and, under Open
  Decision 1's recommended option, every `marshal_cstring` caller). Run
  `cargo test --test golden`. Every diff must trace to a changed helper: objdump
  one per helper.
Commit: —

### Phase 3: Body fixes and the Exempt rows

- [ ] The stream (or arm) fix for each copying body from Phase 1.
- [ ] The 15 rows → `Exempt { reason, proof }` (or `Arm`, per Phase 1). Their
      `cases.tsv` lines → `exempt` (or `arm`).
- [ ] A fixture for each row Phase 1 found untested.
- [ ] RED proof: restore `htmlEscape`'s `MUT out AS String = text` copy. The `exempt`
      bound on its line must fail. Restore the fix.

Acceptance: `cargo test --bin mfb self_update`, the harness over the 15 lines, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/encoding'`
(plus `rt-behavior/net` and `rt-behavior/regex`) → pass (est. 10 min).
Commit: —

## Validation Plan

- Tests: 15 harness lines, the loop cases in Phase 2, fixtures for untested rows.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

1. **Scope of the F4 fix.** Recommended: change `marshal_cstring` itself, so all
   five of its callers (`getEnv`/`getEnvOr`, `setEnv` ×2, `unsetEnv`, `hasEnv`)
   borrow, plus `readText`'s and `canonicalPath`'s own copies. One mechanism, and
   the same proof holds for every caller. The other `fs` path copy loops
   (`gen_directory.rs:605`, `gen_open.rs`, `gen_atomic_write.rs` for non-`readText`
   functions) are not self-update rows. File them with `/write-bug` rather than
   widen this letter.
   Alternative: change only the four rows' call sites.
   DECISION:

## Corrections

## Summary

F turns 15 rows into proven exemptions. First it removes the copies that would
make the proofs false: F4's four C-string copies, and any MFBASIC body that seeds
its output with a copy of its argument, which `htmlEscape` already does.
