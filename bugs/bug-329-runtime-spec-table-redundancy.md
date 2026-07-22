# bug-329: the runtime helper spec catalog is largely redundant — 189 dead lines, a mechanically derivable `symbol` field, a constant `clobbers` field, 133 inert `RuntimeAbiParam` records, and front-end/back-end type tables that already disagree

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup) / Dead-code

Status: Open
Regression Test: new `src/target/shared/runtime/catalog.rs` table-driven parity
test covering all catalogued specs and all 10 families (Phase 1).

`src/target/shared/runtime/` maintains a hand-written catalog of 156 runtime
helper specs across 10 package families. Measured against the working tree, most
of what those specs state is either dead, mechanically derivable, or never read:
one entire 189-line spec file is unreachable and kept compiling only by its own
`#[allow]` attributes; every catalogued spec's `symbol` field is exactly what
`symbol_for_call(helper, call)` produces (156/156, machine-verified); `clobbers`
is the same constant at all 164 spec definitions; and the 133 hand-written
`RuntimeAbiParam` records contribute zero behavior — only `!params.is_empty()` is
ever read. Meanwhile the same package facts are restated in `src/builtins/P.rs`
and `src/target/shared/runtime/P_specs.rs`, and the two copies **already
disagree** in three verified places.

The single correct behavior a fix produces: each fact about a runtime helper is
stated once, in one place, and any fact that is derivable is derived rather than
transcribed — with a machine check proving the derivation before the transcribed
copy is deleted. Nothing about emitted code changes.

References:

- `src/docs/spec/memory/07_runtime-helper-abi.md:19-40` — documents the symbol
  scheme this bug proposes to derive from: `_mfb_rt_<helper>_<call>` with every
  non-`[A-Za-z0-9_]` byte replaced by `_`.
- `src/docs/spec/memory/07_runtime-helper-abi.md:44-46,57` — calls `RuntimeHelperAbi`
  "an explicit, **machine-checkable** description of the calling contract" and
  `RuntimeAbiParam.name` "documentary". The first claim is aspirational: nothing
  machine-checks it (see Current State §3).
- bug-120.1 — made `strings::` ops native-direct, orphaning `strings_specs.rs`.
- Found during the cleanup review (Agent 09 items 2, 3, 10, 14; Agent 17 item 9).

## Current State

Every figure below is measured. The `symbol` derivation and the count figures
come from a throwaway `#[test]` compiled into `catalog.rs`, run, and reverted —
not from parsing source text, so macro-generated specs are included.

### 1. `strings_specs.rs` is 189 lines of fully dead code

```
$ grep -rn "STRINGS_[A-Z_]*_SPEC" src/ | grep -v "runtime/strings_specs.rs"
$ echo "exit=$?"
exit=1                                  # zero references outside the file itself
```

```
$ grep -rn "RuntimeHelper::Strings" src/
src/target/shared/runtime/strings_specs.rs:38,49,60,71,82,93,104,115,126,137,148,159,170,181
src/target/shared/runtime/mod.rs:32:            RuntimeHelper::Strings => "strings",
src/target/shared/runtime/mod.rs:88:// (bug-120.1); the module is retained to avoid a wide `RuntimeHelper::Strings`
```

14 `STRINGS_*_SPEC` constants, 189 lines, zero external references. The
`RuntimeHelper::Strings` variant is *constructed* only inside the dead file; its
sole other appearance is its own `name()` arm at `mod.rs:32`. The module compiles
only because of two attributes it carries for itself
(`src/target/shared/runtime/mod.rs:87-91`):

```rust
// strings:: ops are native-direct, so these specs are no longer catalogued
// (bug-120.1); the module is retained to avoid a wide `RuntimeHelper::Strings`
// enum-variant churn.
#[allow(dead_code)]
mod strings_specs;
```

and `#[allow(unused_imports)] use strings_specs::*;` at `:105-106`. The stated
justification — "to avoid a wide `RuntimeHelper::Strings` enum-variant churn" —
does not hold: the variant has exactly two non-dead-file references, so the churn
is two lines.

`catalog.rs:115-118` carries a five-line comment where the entries used to be.

### 2. `symbol` is derivable — 156/156, machine-verified

A temporary test appended to `catalog.rs` asserting
`spec.symbol == symbol_for_call(spec.helper, spec.call)` for every catalogued
spec:

```
$ cargo test --bin mfb throwaway_verify -- --nocapture
TMP total_specs=156
TMP symbol_mismatches=0 []
TMP distinct_clobbers=1
TMP clobber_group count=156 len=31
TMP total_param_records=214 specs_with_empty_params=37
TMP per_family={"audio": 14, "crypto": 10, "datetime": 3, "fs": 36, "io": 15,
                "net": 20, "os": 15, "term": 17, "thread": 17, "tls": 9}
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2992 filtered out
```

**Zero mismatches across all 156 catalogued specs.** The derivation holds
without exception. (The catalog holds 156 specs, not the 161 the review lead
reported; 161 was a source-regex artifact that missed the 9 macro-generated
`crypto` specs and counted the 14 dead `strings` ones. 156 catalogued + 14 dead
strings = 170 total `RuntimeHelperSpec` constants defined.)

Note `spec_for_symbol` (`catalog.rs:168-172`) does a linear scan comparing
`spec.symbol`; consumers are `code/mod.rs:667` and `:1342`.

### 3. `clobbers` is one constant everywhere; `params` is inert

```
$ grep -h "clobbers:" src/target/shared/runtime/*_specs.rs | sort | uniq -c
   3                 clobbers: abi::IO_PRINT_CLOBBERS,      # inside crypto macro bodies
 161         clobbers: abi::IO_PRINT_CLOBBERS,
```

**164 spec definitions, one distinct value.** The runtime test confirms the
expanded view: `distinct_clobbers=1` across all 156 catalogued specs.
`abi::IO_PRINT_CLOBBERS` is `&["x0", "x1", "x2", "x9", "x16"]`
(`src/target/shared/abi.rs:12`) — an AArch64-spelled register list carried by
every spec including the x86 and riscv paths.

Reads of the three `abi` fields, repo-wide:

```
$ grep -rn "abi\.params" --include='*.rs' src/
src/target/shared/validate.rs:212:                && !spec.abi.params.is_empty()
src/target/shared/runtime/audio_specs.rs:352:        assert!(!spec.abi.params.is_empty());

$ grep -rn "abi\.clobbers" --include='*.rs' src/
src/target/shared/validate.rs:214:                && !spec.abi.clobbers.is_empty()
src/target/shared/runtime/io_specs.rs:209-210    (test)
src/target/shared/runtime/audio_specs.rs:354     (test)

$ grep -rc "abi\.returns" --include='*.rs' src/target/shared/code/mod.rs
48
```

So:

- **`returns` is load-bearing** — read 48 times in `code/mod.rs` to build each
  `CodeFunction`, plus a `!= "Nothing"` test at `:2668`. It stays.
- **`params` is inert.** The only non-test read is `!is_empty()` at
  `validate.rs:212`. The `name`, `type_`, and `location` fields of all 133
  hand-written `RuntimeAbiParam` records are never read by anything.
- **`clobbers` is inert** beyond the same `!is_empty()` at `validate.rs:214`.

The 133 records (`net_specs.rs` alone holds 48) expand to 214 param slots across
the 156 specs, because param arrays are shared between specs.

The gate they feed (`src/target/shared/validate.rs:210-215`):

```rust
        let helper_supported = runtime::supported_helper_specs().iter().any(|spec| {
            spec.helper == *helper
                && !spec.abi.params.is_empty()
                && !spec.abi.returns.is_empty()
                && !spec.abi.clobbers.is_empty()
        });
```

This is an `any()` over the family, so a family passes as soon as **one** of its
specs has non-empty params. 37 of the 156 specs have `params: &[]`; the check is
therefore satisfied by a single sibling and cannot detect a genuinely
under-described helper. `audio_specs.rs:346-348` documents this in a comment that
hardcodes the line reference `"validate.rs:210"`.

### 4. Nine `tls::` specs live in `net_specs.rs`

```
$ grep -n 'call: "tls\.' src/target/shared/runtime/net_specs.rs
532,543,554,565,576,587,598,609,620
```

`TLS_CONNECT_SPEC` … `TLS_CLOSE_LISTENER_SPEC` occupy
`src/target/shared/runtime/net_specs.rs:530-627`, behind a section banner at
`:410-412`. Every other family maps one file to one `RuntimeHelper`; `tls` is the
only break in the rule, and `RuntimeHelper::Tls` is a distinct variant
(`mod.rs:16`) declared alongside all the others.

### 5. Parity tests cover 3 of 10 families, via hand-copied arrays

```
$ grep -ln "#\[test\]" src/target/shared/runtime/*_specs.rs
src/target/shared/runtime/audio_specs.rs
src/target/shared/runtime/os_specs.rs
src/target/shared/runtime/io_specs.rs
```

`audio` (14 specs), `os` (15), `io` (15) are covered — 44 of 156. `fs` (36),
`net` (20), `term` (17), `thread` (17), `crypto` (10), `datetime` (3), `tls` (9)
— 112 specs — have none.

The three that exist are copy-pasted and driven by a hand-maintained call-name
array (`audio_specs.rs:300-313`, `AUDIO_CALLS`). The array duplicates the catalog
it is meant to check: adding a spec without adding its name to the array leaves
the new spec unverified and the test still green.

### 6. Package facts are split and already disagree

Three verified disagreements between `src/builtins/P.rs` (front end, authoritative
— it is what accepts or rejects user code) and
`src/target/shared/runtime/P_specs.rs` (back end):

| Fact | Front end | Back end |
| --- | --- | --- |
| `net.close` param type | `"Socket or Listener or UdpSocket"` — `src/builtins/net.rs:257`; accepted at `:208` (Socket, Listener) and `:236` (UdpSocket) | `type_: "Socket"` — `net_specs.rs:54-58` (`NET_SOCKET_PARAMS`, used by `NET_CLOSE_SPEC` at `:253`) |
| `net.close` param name | `&[&["resource", "sock", "listener"]]` — `src/builtins/net.rs:117` | `name: "sock"` — `net_specs.rs:55` |
| `audio` stream handle type | accepts `AudioInput` **and** `AudioOutput` for `poll`/`available`/`xruns` — `src/builtins/audio.rs:300-311` | `type_: "AudioInput"` — `audio_specs.rs:107-111` (`AUDIO_STREAM_PARAMS`) and `:113-119` |
| `strings` second-param names | `prefix` / `suffix` / `needle` / `delimiter` — `src/builtins/strings.rs:105-108` | `name: "pattern"` — `strings_specs.rs:17-21` (`STRING_VALUE_PATTERN_PARAMS`, used at `:130,141,152,163`) |

The `audio` case is self-contradicting within a single file: the comment
immediately above the record says the handle is *"either direction"*
(`audio_specs.rs:105-106`) while the record types it `AudioInput`.

None of the four disagreements can miscompile today, precisely because the
back-end `type_` field is never read (§3) — which is the point. These fields look
authoritative, are cited by the spec as the calling contract, and are wrong.

## Root Cause

`RuntimeHelperSpec` was designed as a self-describing ABI record and the spec
document (`07_runtime-helper-abi.md:44-46`) describes it as machine-checkable.
The machine check was never written. With nothing reading `params`, `clobbers`,
or the `name`/`type_`/`location` fields, and nothing tying `symbol` to
`symbol_for_call`, each new spec was filled in by copying a neighbor. Copies of
unread fields do not produce failures, so they drift silently — which is exactly
what §6 shows: the `type_` strings track whatever the author of that spec
believed at the time, and the front end has since grown overloads the back-end
records never learned about.

`strings_specs.rs` is the terminal case: bug-120.1 removed the last consumer, and
because nothing read the file, only an `#[allow(dead_code)]` was needed to keep
the tree compiling — the deletion was deferred with a justification
(enum-variant churn) that is not true.

## Goal

- `src/target/shared/runtime/strings_specs.rs` and `RuntimeHelper::Strings` are
  deleted; no `#[allow(dead_code)]`/`#[allow(unused_imports)]` remains in
  `runtime/mod.rs`.
- `RuntimeHelperSpec::symbol` is gone; `spec_for_symbol` and `code/mod.rs`
  consumers derive via `symbol_for_call`. A test proves the derivation for every
  catalogued spec **before** the field is removed.
- `RuntimeHelperAbi::clobbers` and the `RuntimeAbiParam` `name`/`type_`/`location`
  fields are either deleted or made load-bearing — not left as unread
  transcription. (See Open Decisions.)
- The nine `tls::` specs live in `src/target/shared/runtime/tls_specs.rs`.
- One table-driven parity test in `catalog.rs` covers every catalogued spec and
  all 10 families, driven by the catalog itself rather than a hand-copied array.
- Each of the four front-end/back-end disagreements in §6 is resolved by deleting
  the back-end copy, not by editing it to match.

### Non-goals (must NOT change)

- **Emitted output does not change.** Every item here is dead code, a derivable
  field, or an unread field. `scripts/artifact-gate.sh` must show a **zero** diff
  for the entire change, and `scripts/test-accept.sh` must be green. Any artifact
  delta means something read a field that was believed inert — stop and
  investigate.
- The symbol scheme itself. `symbol_for_call` and the doubled-module quirk
  (`io.print` → `_mfb_rt_io_io_print`) are the documented contract
  (`07_runtime-helper-abi.md:19-40`) and stay exactly as they are. This bug
  removes the *transcribed copies*, never the scheme.
- `RuntimeHelperAbi::returns` — read 48 times in `code/mod.rs`. It stays.
- The front-end tables in `src/builtins/*.rs`. They are authoritative; the fix
  deletes the divergent back-end copies.
- Do **not** resolve §6 by editing `net_specs.rs`/`audio_specs.rs` `type_`
  strings to match the front end. That preserves the duplication and guarantees
  the next drift; the fields are unread and must go.
- Do **not** delete `symbol` before the Phase 1 derivation test exists and passes.
  If any future spec ever needs a symbol that is not derivable, the field must
  come back — the test is what makes that discoverable.

## Blast Radius

**Fixed by this bug**

- `src/target/shared/runtime/strings_specs.rs` (189 lines) — deleted.
- `src/target/shared/runtime/mod.rs:14` (`RuntimeHelper::Strings`), `:32`
  (`name()` arm), `:87-91` (`#[allow(dead_code)] mod`), `:105-106`
  (`#[allow(unused_imports)] use`) — deleted.
- `src/target/shared/runtime/catalog.rs:115-118` — the tombstone comment goes
  with the file.
- `src/target/shared/runtime/mod.rs:60` (`symbol` field), all 156 `symbol:` lines
  in the ten `*_specs.rs` files, and the 3 macro `$symbol:literal` parameters in
  `crypto_specs.rs:64-106` — deleted.
- `src/target/shared/runtime/catalog.rs:168-172` (`spec_for_symbol`) — compares
  `symbol_for_call(spec.helper, spec.call)` instead of `spec.symbol`.
- `src/target/shared/code/mod.rs:667`, `:1342` — `spec_for_symbol` consumers;
  signature unchanged, no edit expected. Verify.
- `src/target/shared/runtime/net_specs.rs:410-627` — moved to new `tls_specs.rs`.
- `src/target/shared/runtime/audio_specs.rs:294-356`,
  `os_specs.rs:214-251`, `io_specs.rs:199-212` — replaced by the single
  table-driven test in `catalog.rs`. This also removes the hardcoded
  `"validate.rs:210"` line reference at `audio_specs.rs:346`.

**Latent, same hazard, out of scope**

- `src/target/shared/validate.rs:210-215` — the `any()` gate is structurally
  unable to detect an under-described helper (37 of 156 specs have empty params).
  Out of scope: fixing the gate is a semantic change to capability validation and
  would move this bug off LOW. It must, however, keep compiling after `params`
  and/or `clobbers` are removed — see Open Decisions.
- `src/docs/spec/architecture/06_native.md` — omits `audio` from the helper family
  list (Agent 09 item 8). Separate doc drift, not touched here.
- `src/docs/spec/memory/07_runtime-helper-abi.md:26` — the family list also omits
  `audio`. Same; fix opportunistically when updating the doc for this change.

**Unaffected**

- `src/builtins/*.rs` front-end tables — authoritative and untouched.
- `abi::IO_PRINT_CLOBBERS` (`src/target/shared/abi.rs:12`) — the register
  allocator models call clobbering independently via its own masks (`abi.rs:14-16`);
  removing the spec field does not remove the constant's other users.

## Fix Design

Ordered so that every deletion is preceded by the proof that makes it safe.

**The derivation test comes first, and is permanent.** Phase 1 adds to
`catalog.rs`:

```text
#[test] fn every_spec_symbol_is_derivable() {
    for spec in supported_helper_specs() {
        assert_eq!(spec.symbol, symbol_for_call(spec.helper, spec.call), ...);
    }
}
```

This passes today (156/156, §2). It lands **before** the field is removed, on its
own commit, so that the derivation is a checked-in fact rather than a claim in
this document. When Phase 4 deletes the field, the assertion is rewritten against
the derived symbol's *consumers* — it does not simply vanish.

**The single table-driven parity test** replaces the three hand-copied ones.
Driven by the catalog itself, so a new spec is covered the moment it is added:

```text
#[test] fn catalog_is_consistent() {
    let specs = supported_helper_specs();
    let mut seen_symbols = HashSet::new();
    let mut families    = HashSet::new();
    for spec in specs {
        // family round-trip: the front end routes this call to this helper
        assert_eq!(helper_for_call(spec.call), Some(spec.helper), "{}", spec.call);
        // call round-trip
        assert_eq!(spec_for_call(spec.call).map(|s| s.call), Some(spec.call));
        // symbol round-trip + uniqueness
        let sym = symbol_for_call(spec.helper, spec.call);
        assert_eq!(spec_for_symbol(&sym).map(|s| s.call), Some(spec.call));
        assert!(seen_symbols.insert(sym), "duplicate symbol for {}", spec.call);
        // returns is the one load-bearing abi field
        assert!(!spec.abi.returns.is_empty(), "{} returns", spec.call);
        families.insert(spec.helper);
    }
    assert_eq!(families.len(), 10, "every RuntimeHelper family is catalogued");
}
```

This is strictly stronger than the three tests it replaces: it covers 156 specs
instead of 44, all 10 families instead of 3, and it cannot fall out of date
because there is no array to maintain. The `families.len() == 10` assertion is
what would have caught the `strings` situation — a `RuntimeHelper` variant with
no catalogued spec.

**`tls_specs.rs`** is a lift-and-shift of `net_specs.rs:410-627` plus its
`TLS_*_PARAMS` consts, a `mod tls_specs;` / `use tls_specs::*;` in `mod.rs`
alongside the other nine, and deletion of the section banner from `net_specs.rs`.
No spec contents change.

**Rejected alternatives.**

- *Keep `symbol` and add the assertion as the only fix.* Rejected: the assertion
  is necessary but leaves 156 transcribed strings that must stay in sync by test
  rather than by construction. Derivation is the point; the test is the proof
  that derivation is safe.
- *Fix the §6 disagreements by correcting the back-end `type_` strings.*
  Rejected in Non-goals — it re-establishes the duplication that produced the
  drift.
- *Keep `strings_specs.rs` "in case strings:: helpers return".* Rejected: it has
  been dead since bug-120.1, its own justification comment is factually wrong,
  and git retains it. If strings helpers return they will be written against
  whatever the ABI looks like then.
- *Populate `params`/`clobbers` into a real per-call clobber model instead of
  deleting them.* This is the "make it load-bearing" branch of Open Decisions —
  a genuine feature (`abi.rs:14-16` notes a "correct per-call clobber reader" is
  wanted), but it is a performance/codegen change, not cleanup, and belongs in
  its own plan. It must not ride along here.

## Phases

### Phase 1 — prove the derivation and build the parity net (no behavior change)

- [x] Add `every_spec_symbol_is_derivable` to `catalog.rs`. Confirm it passes
      156/156 today. **This must land before any field is removed.**
      (Passes: `cargo test --bin mfb catalog::`.)
- [x] Add the table-driven `catalog_is_consistent` test. The strings-hole
      reproduction step is moot — bug-326 already deleted `RuntimeHelper::Strings`
      (see Corrections); the family assertion pins the 10 catalogued families
      directly. Found and encoded the code-layer-only seam for
      `thread.drop`/`read`/`emit` (see Corrections).
- [x] Re-verify the read-site census in Current State §3 (`abi.params`,
      `abi.clobbers`, `abi.returns`) and record any drift in this file
      (see Corrections: counts moved by the bug-326 strings deletion; the §2
      `symbol` consumer census was incomplete).

Acceptance: the derivation test is green and checked in; the parity test
demonstrates the `strings` hole; the census is current.
Commit: —

### Phase 2 — delete the dead strings catalog

**Already landed by bug-326 (commit 8ec1872b6) before this bug was worked — see
Corrections.** Verified against the working tree:

- [x] Delete `src/target/shared/runtime/strings_specs.rs`. (Done by bug-326-A1;
      `grep -rn "Strings\|strings_specs\|STRINGS_" src/target/` → no remnants.)
- [x] Delete `RuntimeHelper::Strings`, its `name()` arm, and both `#[allow]`
      attributes. (Done by bug-326; `grep -rn "allow(dead_code)\|allow(unused"
      src/target/shared/runtime/` → empty.)
- [x] Tombstone comment at `catalog.rs`: bug-326 rewrote it into deliberate
      documentation of why no `strings::` row exists — kept, not deleted.
- [x] `catalog_is_consistent` lands un-ignored; the 10-family assertion passes.

Acceptance: met by bug-326; parity test green in Phase 1.
Commit: 8ec1872b6 (bug-326)

### Phase 3 — move the TLS specs

- [x] Create `src/target/shared/runtime/tls_specs.rs` from
      `net_specs.rs:415-627` plus the `TLS_*_PARAMS` consts; wire `mod`/`use` in
      `mod.rs`. (Moved block verified byte-identical:
      `git show HEAD:...net_specs.rs | sed -n '415,627p' | diff - <(sed -n
      '9,221p' tls_specs.rs)` — empty.)
- [x] Remove the TLS section and banner from `net_specs.rs`.

Acceptance: pure move (diff-verified); parity test green; artifact gate covered
by the whole-change run in Phase 6 (no spec content changed).
Commit: (see git log — "bug-329 phase 3")

### Phase 4 — remove the derivable `symbol` field

- [x] Delete `symbol` from `RuntimeHelperSpec`, all 147 literal `symbol:` lines
      (156 minus the 9 macro-generated), and the `$symbol:literal` parameter
      from the three `crypto_specs.rs` macros plus its 9 invocation arguments.
      A comment on the struct records why the field is gone and when it must
      come back.
- [x] Rewrite `spec_for_symbol` to compare against
      `symbol_for_call(spec.helper, spec.call)`.
- [x] Verify the `spec_for_symbol` consumers (`code/mod.rs:772`, `:1479` in the
      current tree) need no change — verified, they pass a `&str` and read
      `spec.call`; no edit made.
- [x] The plan-layer consumers the original census missed (see Corrections)
      rewritten: `linux_common/plan.rs` and `macos_aarch64/plan.rs`
      `runtime_imports` derive the symbol once at function entry;
      `shared/plan/mod.rs` test platform likewise; `shared/plan/symbols.rs`
      derives at its two spec sites and `os_env_lock_helper_symbols` returns
      `Vec<String>`.
- [x] `every_spec_symbol_is_derivable` folded into `catalog_is_consistent`,
      whose symbol round-trip over the derived symbol
      (`spec_for_symbol(symbol_for_call(...)) → same spec` + uniqueness) is the
      surviving form of the guarantee, marked as such in a comment.

Acceptance: parity test green; artifact gate zero delta (whole-change run in
Phase 6; the derived string was proven byte-identical by the Phase 1 test
before deletion).
Commit: (see git log — "bug-329 phase 4")

### Phase 5 — resolve `params` / `clobbers` per Open Decisions

- [ ] Apply the decision recorded below. If deleting: remove the fields, remove
      the 133 `RuntimeAbiParam` records, and adjust `validate.rs:210-215` to gate
      on `returns` alone — documenting in that function why the params/clobbers
      conditions went away.
- [ ] Confirm the four §6 disagreements are gone *by deletion*, and that
      `src/builtins/net.rs`, `audio.rs`, `strings.rs` were not edited.

Acceptance: artifact gate zero delta; `validate.rs` still rejects an
unimplemented helper family (add a unit test for that if none exists).
Commit: —

### Phase 6 — doc sync + full validation

- [ ] Update `src/docs/spec/memory/07_runtime-helper-abi.md`: remove `strings`
      from the family narrative, drop the `RuntimeAbiParam` block if the fields
      were deleted, and either substantiate or soften the "machine-checkable"
      claim at `:44-46` — the new `catalog.rs` test is what makes it true.
      Opportunistically add the missing `audio` family at `:26`.
- [ ] Run `scripts/artifact-gate.sh` — must be **empty**.
- [ ] Run `scripts/test-accept.sh` in full.

Acceptance: full suite green; artifact gate shows zero delta across all phases;
spec and code agree.
Commit: —

## Validation Plan

- Regression tests: `catalog.rs::every_spec_symbol_is_derivable` (Phase 1, the
  gate on Phase 4) and `catalog.rs::catalog_is_consistent` (156 specs, 10
  families, replacing the three hand-copied per-package tests).
- Runtime proof: `scripts/test-accept.sh` full run. Every item is dead or unread,
  so program behavior must be bit-for-bit unchanged.
- Artifact proof: `scripts/artifact-gate.sh` after **every** phase, expecting a
  zero diff each time. A non-zero diff falsifies the "inert field" premise and is
  a stop condition.
- Doc sync: `src/docs/spec/memory/07_runtime-helper-abi.md` (family list,
  `RuntimeAbiParam` block, the "machine-checkable" claim).
- Full suite: `scripts/artifact-gate.sh` + `scripts/test-accept.sh`.

## Open Decisions

- **Delete `clobbers` and the `RuntimeAbiParam` fields, or make them
  load-bearing?** Recommended: **delete**. They are unread today, three of the
  four §6 disagreements live in them, and `abi.rs:14-16` records that the
  register allocator already models call clobbering via its own masks — so the
  spec copy is not the missing piece of any real feature. The alternative
  (implement a per-call clobber reader so the field starts mattering) is a
  codegen change with real performance upside and belongs in its own plan; it
  must not ride along in a LOW cleanup. If that plan is imminent, keep
  `clobbers` and delete only the `RuntimeAbiParam` fields.
- **Keep `RuntimeAbiParam` as documentation with a machine check instead of
  deleting?** Recommended: **no** for `type_` (it is the drift surface — the
  front end owns argument types), **acceptable** for `location`, which encodes a
  genuine positional-register fact not stated elsewhere. If `location` is kept,
  add a test asserting it equals `abi::ARG[i]` for its index, which makes it
  derivable too — and therefore also deletable.
- **Delete `RuntimeHelper::Strings` outright, or keep the variant?** Recommended:
  **delete**. Two references (§1); the "wide enum-variant churn" the comment
  warns of does not exist.

## Corrections (2026-07-22, re-verified before implementation)

- **Phase 2 already landed via bug-326** (commit 8ec1872b6, after this document
  was written): `strings_specs.rs`, `RuntimeHelper::Strings`, and both `#[allow]`
  attributes are gone. `grep -rn "Strings\|strings_specs\|STRINGS_" src/target/`
  finds zero spec remnants. The `catalog.rs` tombstone comment was *rewritten* by
  bug-326 into an informative note citing bug-326-A1 (why no `strings::` row
  exists); it is deliberate documentation now, and is kept rather than deleted.
- **The §2 note on `symbol` consumers was incomplete.** Beyond `spec_for_symbol`,
  the plan layer reads `spec.symbol` directly: `linux_common/plan.rs`
  `runtime_imports` (~100 uses as the `required_by` string),
  `macos_aarch64/plan.rs` `runtime_imports` (~70 uses of
  `spec.symbol.to_string()`), the test platform in `shared/plan/mod.rs` (3), and
  `shared/plan/symbols.rs` (`runtime_symbols` term auto-restore push +
  `os_env_lock_helper_symbols`). Command:
  `grep -rn "spec\.symbol" --include='*.rs' src/`. Phase 4 rewrites each function
  to derive the symbol once via `symbol_for_call`; `os_env_lock_helper_symbols`
  changes return type `Vec<&'static str>` → `Vec<String>`.
- **Counts after the bug-326 deletion:** 150 `clobbers:` definition sites (147
  literal + 3 in crypto macro bodies), still exactly one distinct value
  (`grep -h "clobbers:" src/target/shared/runtime/*_specs.rs | sort | uniq -c`).
  `abi.returns` is read 46 times in `code/mod.rs` (was 48). The `spec_for_symbol`
  consumers sit at `code/mod.rs:772` and `:1479`.
- **`RuntimeHelper` grew `General` and `Math` variants** (not in this document's
  10-family model). Both are fully native-direct — no catalogued spec, no
  `_mfb_rt_*` emission — so the Phase 1 parity test pins the catalogued family
  set to the 10 real ones and documents those two exceptions. The doc's
  "families.len() == 10 fails while Strings exists" reproduction step is moot:
  bug-326 already removed the hole.
- **The Fix Design's `helper_for_call(spec.call) == Some(spec.helper)` assertion
  is too strong for four specs.** `thread.drop`, `thread.read`, `thread.emit`,
  and `net.connectTcpAddr` are synthesized inside the code layer
  (`builder_values` rewrites the user call into the direction/overload-specific
  variant; `drop` is emitted by codegen primitives), so they never exist at the
  NIR level where `helper_for_call` routes calls — the classifier deliberately
  returns `None` for them (found by running the test; the collected misroute
  list is empty with exactly these four excluded). The landed
  `catalog_is_consistent` pins this seam both ways: those four must be
  unclassified, every other spec must classify to its helper.
- The three per-family test modules: the audio/os hand-array parity tests were
  deleted in Phase 1, replaced by the catalog-driven test (strict superset for
  catalogued specs). `audio_family_is_complete_for_validate` (params/clobbers
  gate assertion) and io's bug-70 `io_flush_declares_call_clobbers` stay until
  Phase 5 deletes the fields they assert on — bug-70's protected behavior
  ("never declare a false empty clobber set") becomes vacuously impossible once
  the field itself is gone; the real clobber model lives in the register
  allocator (`regalloc/analysis.rs` call-clobber masks), unchanged.

## Summary

The engineering risk here is unusually low and concentrated entirely in Phase 4's
premise — that `symbol` is derivable — which is why Phase 1 lands a permanent,
checked-in test proving it 156/156 before any field is deleted. Everything else
is removal of code that nothing reads: 189 dead lines, 156 transcribed symbol
strings, one constant repeated 164 times, and 133 parameter records whose only
observable contribution is a `!is_empty()` predicate that an `any()` over the
family satisfies anyway. The lasting value is the single table-driven parity test
replacing three hand-copied arrays: it covers 156 specs and all 10 families
instead of 44 and 3, and its family-completeness assertion is what would have
caught the dead `strings` catalog years ago. The front-end tables in
`src/builtins/`, the symbol scheme, `abi.returns`, and all emitted output are
untouched.
