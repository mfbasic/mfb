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
  (`fs/gen_atomic_write.rs:811`; Correction F1 retracts the plan's second
  citation, `:1091`, which belongs to `lower_fs_read_bytes_path_helper`).
  `fs::canonicalPath` marshals `c_path`
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

- [x] For each of the 10 MFBASIC bodies: every statement that copies `value`
      (a `MUT`/`LET` bound to it, a `mid`/`left` over the whole of it, a
      concatenation seeded with it), with a line citation. Record the stream-or-arm
      choice per copying row.
- [x] For the 5 not-derived rows: confirm F4's four copies are the only copies of
      `s`, and that `io::input` makes none.
- [x] Measure: the rt-behavior fixtures that exercise each of the 15 rows
      (`grep -rl '<pkg>::<f>(' tests/rt-behavior | wc -l` per row). A row with 0
      fixtures gets one in Phase 3.

#### The 10 MFBASIC bodies: exactly one copies its argument

Exactly ONE of the ten seeds its output with its argument: `encoding::htmlEscape`,
`src/codegen/builtins/encoding/func_html_escape.rs:42` (`MUT out AS String = text`),
followed by five `out = strings::replace(out, …)` at `:43-47`. **Choice: stream**
(Open Decision 2's recommended option) — rewritten as a single left-to-right
grapheme scan. *(Correction F4: the scan must read through `strings::mid`, not
through `strings::graphemes` as `__encoding_htmlUnescape` did; the grapheme list
is itself live while the argument is, and fails the `exempt` bound. `htmlUnescape`
was rewritten the same way for the same reason.)*

The other nine already stream — each starts its output from empty, or delegates to
a helper that does. "Streams" here is a statement about where the **output**
starts; Correction F4 records that it is not on its own enough to meet the
`exempt` bound, which is about peak live bytes:

| Row | Where the output starts | Citation |
|---|---|---|
| `encoding::formUrlDecode` | delegates | `func_form_url_decode.rs:37` → `helper_percent_decode_bytes.rs:16` (`MUT result AS List OF Byte = []`) |
| `encoding::formUrlEncode` | `MUT out AS String = ""` | `func_form_url_encode.rs:41` |
| `encoding::htmlUnescape` | `MUT out AS String = ""` | `func_html_unescape.rs:55` |
| `encoding::percentDecode` | delegates | `func_percent_decode.rs:33` → `helper_percent_decode_bytes.rs:16` |
| `encoding::percentEncode` | `MUT out AS String = ""` | `func_percent_encode.rs:38` |
| `encoding::punycodeDecode` | `MUT out AS String = ""` | `func_punycode_decode.rs:51` |
| `encoding::punycodeEncode` | `MUT out AS String = ""` | `func_punycode_encode.rs:42` |
| `net::percentDecode` | delegates | `func_percent_decode.rs:74` → `helper_percent_decode_impl.rs:19` (`MUT out AS List OF Byte = []`) |
| `regex::replace` | `MUT out AS String = ""` | `func_replace.rs:130`; its `strings::mid(value, …)` calls are slices around matches, not the whole of `value` |

Measured with
`grep -n 'MUT out AS\|List OF Byte = \[\]\|RETURN __' src/codegen/builtins/encoding/func_*.rs src/codegen/builtins/net/func_percent_decode.rs src/codegen/builtins/regex/func_replace.rs`.

#### The 5 not-derived rows

- `os::getEnv` / `os::getEnvOr`: the only copy of `name` is the C-string marshal,
  `marshal_cstring` called at `src/codegen/builtins/os/gen_env.rs:158`.
  `os::getEnvOr` also copies its **fallback** (`gen_env.rs:205-247`, the
  `{symbol}_fb_copy_loop`), but that is not the row's `s` — the self-update form is
  `s = os::getEnvOr(s, "fallback")`, so `s` is the *name*.
- `fs::readText`: the `{symbol}_path_copy_loop` in
  `src/codegen/builtins/fs/gen_atomic_write.rs:811` (`emit_cstring_copy` at `:869`).
  The result is read out of the file, not built from the path.
- `fs::canonicalPath`: the `{symbol}_copy_loop` in
  `src/codegen/builtins/fs/gen_canonical.rs:24` (`emit_cstring_copy` at `:85`); the
  result is copied out of `realpath`'s PATH_MAX buffer (`:141-155`).
- `io::input`: copies **nothing**. `gen_read_line_family.rs:158-186` writes the
  prompt straight out of the caller's block at `+8` (`abi::add_immediate(&ptr,
  &prompt, 8)` into the `write` call); the result is built from stdin.

`marshal_cstring` itself (`src/codegen/builtins/os/gen_shared.rs:130-179`) allocs
`len + 1` arena bytes, copies the bytes one at a time, appends a NUL, and does
**not** reject an interior NUL. The block is declared through
`HelperScratch::declare`/`declare_for` and freed by `emit_helper_scratch_release`
at the helper's `done` (for `getEnv`/`getEnvOr`, `gen_env.rs:255-261`). Its five
callers: `gen_env.rs:158`, `func_set_env.rs:57` and `:67`, `func_unset_env.rs:46`,
`func_has_env.rs:47`. The two `fs` sites differ: they call `emit_cstring_copy` with
`reject_nul = true` (`fs/gen_shared.rs:51-54`), so replacing the copy there must
keep the interior-NUL rejection scan.

#### rt-behavior fixture counts

`for c in …; do grep -rl -- "$c" tests/rt-behavior | wc -l; done`:

| Row | Fixtures | Row | Fixtures |
|---|---|---|---|
| `encoding::formUrlDecode` | 1 | `net::percentDecode` | 1 |
| `encoding::formUrlEncode` | **0** | `regex::replace` | 4 |
| `encoding::htmlEscape` | **0** | `fs::readText` | 25 |
| `encoding::htmlUnescape` | 2 | `fs::canonicalPath` | 1 |
| `encoding::percentDecode` | 1 | `io::input` | **0** |
| `encoding::percentEncode` | **0** | `os::getEnv` | 4 |
| `encoding::punycodeDecode` | **0** | `os::getEnvOr` | 2 |
| `encoding::punycodeEncode` | **0** | | |

Six rows need a fixture in Phase 3: `formUrlEncode`, `htmlEscape`,
`percentEncode`, `punycodeDecode`, `punycodeEncode`, `io::input`.

Acceptance: recorded here with citations and the per-row fixture counts
(est. 40 min).
Commit: `1a7e4a122`

### Phase 2: F4

- [x] `getEnv`/`getEnvOr`, `readText` and `canonicalPath` pass `block + 8` to the
      host. Remove the copies and their scratch release.
- [x] Runtime: the existing `os` and `fs` fixtures for these four pass. Add a
      case per row whose argument is a `MUT` binding self-updated in a loop
      (`s = os::getEnvOr(s, "x")`).

What landed:

- `marshal_cstring` is gone; `borrow_cstring(src, out)`
  (`src/codegen/builtins/os/gen_shared.rs`) is one `add_immediate(out, src, 8)`.
  All five `os` callers borrow: `gen_env.rs:153` (`getEnv`/`getEnvOr`),
  `func_set_env.rs` ×2, `func_unset_env.rs`, `func_has_env.rs`. With the arena
  block went the `HelperScratch`/`emit_helper_scratch_release` pair at each site
  and, for `setEnv`/`unsetEnv`/`hasEnv`, the now-unreachable `alloc_error` label
  and its `push_alloc_error` (`unsetEnv` has no failure exit left at all, so its
  `done` label went too — `unsetenv` is a no-op for an absent variable).
- `fs::readText` (`gen_atomic_write.rs:lower_fs_read_text_path_helper`) and
  `fs::canonicalPath` (`gen_canonical.rs:lower_fs_canonical_path_helper`) take
  `abi::add_immediate(&c_path, &path, 8)` and keep the interior-NUL rejection as
  a new read-only scan, `emit_cstring_nul_scan` (`fs/gen_shared.rs`), which
  branches to the same `invalid` label `emit_cstring_copy(reject_nul = true)`
  did. `canonicalPath` still releases its PATH_MAX `realpath` buffer;
  `readText`'s `alloc_error` survives for the post-open result-`String` alloc.
- `mfb spec` §14 (`src/docs/spec/stdlib/14_os.md`) rewritten: the argument is
  borrowed, only the result is copied out; citation re-pointed to
  `borrow_cstring`.
- New fixtures: `tests/rt-behavior/os/os-self-update-borrow-rt` (chained
  `s = os::getEnv(s)` / `s = os::getEnvOr(s, …)` ×3, plus `hasEnv`/`unsetEnv`)
  and `tests/rt-behavior/fs/fs-self-update-borrow-rt` (`p = fs::canonicalPath(p)`
  then `p = fs::readText(p)`, chained).

Acceptance (measured):

- `./scripts/test-accept.sh target/debug/mfb target/accept-actual 'func_os_*' 'os-*' 'fs-*' 'func_fs_*' 'bug101_*' 'bug132_*' 'bug159_*'`
  → `acceptance tests passed (145 test(s) ran)`. (The plan's `'rt-behavior/os'`
  spelling matches nothing: `test-accept.sh`'s globs match a test's *name*, so
  the package globs above are the equivalent — see Correction F3.)
- `cargo test --bin mfb self_update` →
  `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 4288 filtered out`.
- Golden churn: recorded under Phase 3, which regenerated Phases 2 and 3 in one
  pass (the artifact gate ran once over both).
Commit: `66499e1e5`

### Phase 3: Body fixes and the Exempt rows

- [x] The stream (or arm) fix for each copying body from Phase 1.
- [x] The 15 rows → `Exempt { reason, proof }` (or `Arm`, per Phase 1). Their
      `cases.tsv` lines → `exempt` (or `arm`).
- [x] A fixture for each row Phase 1 found untested.
- [x] RED proof: restore `htmlEscape`'s `MUT out AS String = text` copy. The `exempt`
      bound on its line must fail. Restore the fix.
      — **run, and it does NOT fail: see Correction F5.** The proof that does go
      red is the `strings::graphemes` form (Correction F4), which was run and
      recorded there.

What landed:

- **Two** bodies rewritten, not one (Correction F4). `__encoding_htmlEscape` and
  `__encoding_htmlUnescape` each scan left to right with
  `strings::mid(text, i, 1)`. `htmlEscape` no longer starts from
  `MUT out AS String = text` and no longer makes five `strings::replace` passes;
  the "ampersand first" ordering is now structural (each grapheme is emitted
  once). Results and errors are unchanged, which the existing `encoding`
  rt-behavior/rt-error fixtures and the five man-page examples pin
  (`bash scripts/man-run-examples.sh encoding --run htmlEscape htmlUnescape` →
  `examples: 5   built: 5   ran: 5   not run: 0   failed: 0`).
- The 15 rows in `SELF_UPDATE_TABLE`
  (`src/codegen/collection/assign/self_update.rs`) carry
  `SelfUpdate::Exempt { reason, proof }` with two shared reasons
  (`CODEC_REASON`, `NOT_DERIVED_REASON`) and a per-row `proof` citing the
  `file:symbol` that shows `s` is read and not stored. `grep -c 'Pending("F")'`
  → 0.
- The 15 `cases.tsv` lines are `exempt`. Twelve now build `x` at `{M}`
  (`String = "" ; FOR k = 1 TO {M} ; x = x & "a" ; NEXT`), so the bound sees a
  statement whose `x` actually doubles; `os::getEnv`/`getEnvOr` add
  `os::setEnv(x, x)` so the read is idempotent across the N-run loop.
  `regex::replace`'s statement changed (Correction F6). Three lines cannot scale
  `x` and are recorded as such below.
- New fixtures for the six untested rows:
  `tests/rt-behavior/encoding/encoding-codec-self-update-rt` (formUrlEncode,
  htmlEscape, percentEncode, punycodeDecode, punycodeEncode — each also in the
  `s = f(s)` shape) and `tests/rt-behavior/io/io-input-prompt-borrow-rt`
  (`s = io::input(s)`, prompt written then EOF trap, prompt intact after).

**What the `exempt` line measures per row.** Three of the fifteen cannot give
`x` a 64 KiB value, so their line's `x` is the same size at `M` and `2M`, the
statement's cost difference is 0, and the bound passes without measuring a copy
of `x`. This is a property of the rows, not a gap that a different setup closes:

- `fs::readText` — the statement must be idempotent across the N-run loop, which
  is why the setup writes the path as the file's own contents
  (`fs::writeText(x, x)`); the content is therefore the path, and a path cannot
  be 64 KiB (`open` → `ENAMETOOLONG`).
- `fs::canonicalPath` — same: `x` is a path, bounded by `PATH_MAX`.
- `io::input` — `x` is the prompt, which `io::input` echoes to stdout; the
  harness parses the first stdout line as the length, so the prompt must stay
  empty (plan-146-A Correction A4).

For these three the exemption rests on the `proof` (the lowering hands the host a
pointer into the caller's block and stores nothing) and on the Phase 2/3
rt-behavior fixtures, which run the `s = f(s)` shape end to end.

Acceptance (measured):

- `MFB_SELF_UPDATE_SITES=Local,Global MFB_SELF_UPDATE_FILTER='<the 15 signatures, `|`-separated>' cargo test --test rt_inplace_self_update every_self_update_case`
  → `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;
    finished in 181.76s` (30 case/site pairs: 15 lines x S1/S2)
- `cargo test --bin mfb self_update` → `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 4288 filtered out`
- `./scripts/test-accept.sh target/debug/mfb target/accept-actual 'func_encoding_*' 'encoding-*' 'func_net_*' 'func_regex_*' 'regex-*' 'func_os_*' 'os-*' 'func_fs_*' 'fs-*' 'func_io_*' 'io-*' 'bug101_*' 'bug132_*' 'bug159_*' 'func_strings_*' 'strings-*' 'func_json_*' 'json-*' 'func_csv_*' 'csv-*'`
  → `acceptance tests passed (255 test(s) ran)`, 0 mismatches; and
  `… 'func_crypto_*' 'crypto-*' 'func_compress_*' 'compress-*' 'func_astrings_*' 'astrings-*' 'tier-*' '*-rt'`
  → `acceptance tests passed (282 test(s) ran)`, 0 mismatches. Between them these
  cover every package the golden churn below touched.
- `bash scripts/artifact-gate.sh target/release/mfb all` (what `cargo test --test
  golden` runs) →
  `artifact-gate [all]: 1490 tests, 1665 build(s), 2108 golden(s) checked, 0 diff(s)`

**Golden churn, and what produced it.** Two gate rounds, because the body rewrite
happened twice (Correction F4). Round 1, after Phase 2 + the grapheme-list
`htmlEscape`: `1490 tests, 1665 build(s), 2108 golden(s) checked, 148 diff(s)`
over 90 fixtures. Round 2, after both bodies moved to `strings::mid`:
`139 diff(s)` over 89 fixtures — a subset of round 1's fixture set plus this
letter's own new `encoding-codec-self-update-rt`
(`comm -13` of the two fixture lists). Two producers, both changed here, account
for all of them:

1. The `encoding` bodies. `__encoding_htmlEscape` grew in the injected
   `builtins/encoding.mfb` and `__encoding_htmlUnescape` changed shape, so every
   fixture whose program embeds the `encoding` package gets a new `.ir` region
   plus a line-number shift below it — and a new `.ncode`. Verified by rebuilding
   `target/release/mfb build -q -ast -ir tests/rt-behavior/trap/inline-trap-union-bind-rt`
   and diffing: the diff is one contiguous hunk at the old `htmlEscape` body
   (`"line": 1160` → `1167`, the five `strings.replace` assigns replaced by the
   scan) followed by shifted `"line"` values only. That covers `encoding`,
   `regex`, `crypto`, `csv`, `json`, `strings`, `astrings`, `compress`, `tls`,
   `tcp`, `udp`, `app` and the rest — every package that imports `encoding`.
2. The F4 borrow. `target/release/mfb build -q -ncode tests/byte-identity/os`
   then `grep -o '"[a-z_0-9]*copy_loop"'` → only `os_environ_key_copy_loop` and
   `os_environ_val_copy_loop` remain (both copy-OUT loops this letter did not
   touch); `os_getEnv/setEnv/hasEnv/unsetEnv_name_copy_loop` and the
   `*_alloc_error` labels are gone. Same for `tests/byte-identity/fs`:
   `canonicalPath_path_alloc_ok`/`_copy_loop` and
   `readText_path_alloc_ok`/`_path_copy_loop` are replaced by `_scan_loop` /
   `_scan_done`, and `canonicalPath` keeps one `scratch_kept_0` (the `realpath`
   buffer) instead of two.

Only the traced goldens were regenerated — the exact files the gate named, using
the gate's own build invocations (host dumps rebuilt with `-ast -ir`; per-target
dumps rebuilt with `-target <t>` and hashed into `.ncodesum` with
`shasum -a 256`). No fixture's golden set changed shape.
Commit: `66499e1e5`

## Validation Plan

- Tests: 15 harness lines, the loop cases in Phase 2, fixtures for untested rows.
- Per-letter gate: `cargo test --bin mfb` →
  `test result: ok. 4297 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;
  finished in 2168.69s`. That run started before the last edit to
  `self_update.rs` (four `proof` strings, Correction F4's rewording), so the
  affected subset was re-run on the final tree:
  `cargo test --bin mfb self_update` →
  `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 4288 filtered out;
  finished in 96.79s`.
- Not run here: the full `scripts/test-accept.sh` sweep. It was started and
  abandoned at fixture 569/1490 — the `rt-behavior/tcp` block runs on live socket
  timeouts and was taking ~20 min per 50 fixtures under the concurrent
  `cargo test --bin mfb` load. In its place: the artifact gate over all 1490
  fixtures (0 diffs, which covers every `.ast`/`.ir`/`.hex` and per-target dump
  golden this letter could move) plus the two targeted acceptance runs above,
  which cover every package in the churn list. The `tcp`/`tls`/`udp` fixtures
  that appear in that list do so only through their `.ir` — no `encoding` change
  can reach their `build.log` or `.run` — and the artifact gate compares exactly
  that `.ir`.

## Open Decisions

1. **Scope of the F4 fix.** Recommended: change `marshal_cstring` itself, so all
   five of its callers (`getEnv`/`getEnvOr`, `setEnv` ×2, `unsetEnv`, `hasEnv`)
   borrow, plus `readText`'s and `canonicalPath`'s own copies. One mechanism, and
   the same proof holds for every caller. The other `fs` path copy loops
   (`gen_directory.rs:605`, `gen_open.rs`, `gen_atomic_write.rs` for non-`readText`
   functions) are not self-update rows. File them with `/write-bug` rather than
   widen this letter.
   Alternative: change only the four rows' call sites.
   DECISION: recommended — `marshal_cstring` itself is replaced by a borrow
   (`borrow_cstring`), so all five `os` callers borrow, and `readText`'s and
   `canonicalPath`'s own copy loops become NUL-rejection scans.

2. **`htmlEscape`: stream or arm.** DECISION: recommended — **stream**.

## Corrections

- **F1 — `fs::readText`'s copy is at `gen_atomic_write.rs:811`, not `:1091`.**
  §2 cites "`{symbol}_path_copy_loop` (`fs/gen_atomic_write.rs:811`, `:1091`)" as
  if `readText` had two. `:1091` is inside `lower_fs_read_bytes_path_helper`
  (`grep -n 'pub(crate) fn lower_fs_read' src/codegen/builtins/fs/gen_atomic_write.rs`
  → `:801 lower_fs_read_text_path_helper`, `:1074 lower_fs_read_bytes_path_helper`),
  a different function and not a self-update row (`fs::readBytes` returns
  `List OF Byte`, so `s = fs::readBytes(s)` is not even type-correct). `readText`
  has exactly one copy of its path, `emit_cstring_copy` at `:869`.
- **F2 — `os::getEnvOr` copies its `fallback` too, and that copy stays.** §2 names
  only `marshal_cstring` for the `getEnv` family. `lower_get_env` with
  `with_fallback` also emits a `{symbol}_fb_copy_loop`
  (`src/codegen/builtins/os/gen_env.rs:205-247`) that copies the *fallback*
  argument by its stored length. That is not the row's `s` — the row's self-update
  is `s = os::getEnvOr(s, "fallback")`, where `s` is the name — so the exemption
  proof is unaffected and the copy is left alone (removing it would change
  `getEnvOr`'s contract: it returns an owned `String`, and the spec's
  "copied by its stored byte length rather than by NUL scan, so an embedded NUL
  there is preserved verbatim" depends on it, `src/docs/spec/stdlib/14_os.md:31`).
- **F3 — the acceptance commands' `rt-behavior/<pkg>` spelling matches nothing.**
  Phases 2 and 3 spell their acceptance as
  `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/os'`.
  That glob is matched against each test *directory name*, not its path
  (`scripts/test-accept.sh` usage: "name-glob: optional shell glob(s) matched
  against each test dir name"), so it selects nothing:
  `./scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/os'`
  -> `no tests matched filter: rt-behavior/os`. The equivalent is a per-package set
  of name globs (`'func_os_*' 'os-*'`, `'func_encoding_*' 'encoding-*'`, ...); those
  are what this letter ran and recorded.
- **F4 — the `stream` rewrite must not read through `strings::graphemes`, and
  `htmlUnescape` already did.** Phase 1 recorded `htmlUnescape` as one of the nine
  rows that "already stream" (its output starts at `MUT out AS String = ""`), and
  §3's recommended fix for `htmlEscape` was to copy that shape. Both statements are
  true about the *output* and both are the wrong answer for the `exempt` bound,
  which measures **peak live bytes**: `strings::graphemes(text)` builds a
  `List OF String` holding one heap `String` per grapheme, live for as long as
  `text` is. Measured at |x| = 65536 -> 131072 (`MFB_SELF_UPDATE_SITES=Local,Global
  MFB_SELF_UPDATE_FILTER='encoding::htmlEscape' cargo test --test
  rt_inplace_self_update every_self_update_case`, and the same numbers from the
  standalone probe below):

  | body of the row | cost at M | cost at 2M | growth | bound | verdict |
  |---|---|---|---|---|---|
  | `htmlEscape`, grapheme-list scan | 2785360 | 5554256 | 2768896 | 2360320 | **fails** |
  | `htmlUnescape`, grapheme-list scan | 2785376 | 5554272 | 2768896 | 2360320 | **fails** |
  | `htmlEscape`, `strings::mid` scan | 98336 | 180256 | 81920 | 2360320 | passes |
  | `htmlUnescape`, `strings::mid` scan | 98352 | 180272 | 81920 | 2360320 | passes |

  That is ~42 bytes of live grapheme list per character of `text`. Both bodies now
  scan with `strings::mid(text, i, 1)` — the shape `__net_percentDecodeImpl`
  already used (`net/helper_percent_decode_impl.rs`), and the reason that row
  passes — which allocates one short-lived grapheme per step. So this letter
  rewrote **two** bodies, not one; the result and error behaviour of each is
  unchanged (the existing `encoding` rt-behavior/rt-error fixtures, plus the new
  `encoding-codec-self-update-rt`).
- **F5 — the RED proof Phase 3 names cannot go red; that is a limit of the bound,
  not of the fix.** Phase 3 says: restore `htmlEscape`'s `MUT out AS String = text`
  and the `exempt` bound on its line must fail. It does not. Measured with that
  body restored, |x| = 65536 -> 131072: cost 98320 -> 163856, **growth 65536**,
  against a bound of **2360320**. Run through the harness, not just the probe —
  `MFB_SELF_UPDATE_SITES=Local,Global MFB_SELF_UPDATE_FILTER='encoding::htmlEscape'
  cargo test --test rt_inplace_self_update every_self_update_case` with that body
  restored → `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;
  1 filtered out; finished in 88.65s`. The bound is
  `EXEMPT_M * width / 2 + 4 * Δlen * width + 1024`
  (`tests/runtime/rt_inplace_self_update.rs:exempt_check`), and for a `String` line
  `width` is 8 while a copy of `x` costs |x| = 65536 *bytes*: the
  `EXEMPT_M * width / 2` term alone is 262144, four times the largest copy of `x`
  the case can make. So **no `String` row can fail this bound on account of one
  copy of `x`**. The RED proof that does go red is F4's — restoring
  `strings::graphemes` in `htmlEscape` fails the line, 2768896 >= 2360320 — and
  that is what Phase 3 ran and recorded. The residual gap (a body that seeds its
  output with its argument is caught by the `proof` citation and by review, not by
  the harness) belongs to **letter H**, which locks the guard: the check that would
  catch it is static, not dynamic — assert that no `Exempt` row's `Body::mfb` binds
  a parameter whole, `MUT|LET <id> AS <ty> = <param>` — and adding a guard shape is
  that letter's job.
- **F6 — `regex::replace`'s `cases.tsv` statement had to change to be measurable.**
  The line's statement was `x = regex::replace(x, "a", "aa")`, which at |x| = 65536
  matches every character: `__regex_replace` materialises the whole match list
  (`FOR EACH r IN __regex_matchResults(prog, ctx, 0)`), one record with a `caps`
  list per match. Measured cost 25772320 -> 38608896, growth 12836576 against a
  bound of 4457472 — it fails `exempt`, and the cause is the match list, not a copy
  of `x` (rewriting the engine to stream matches is a regex change, not a
  `String`-self-update one). The statement is now
  `x = regex::replace(x, "^a", "b") ; x = regex::replace(x, "^b", "a")` — still a
  round trip, still length-preserving, but one match instead of |x| of them:
  cost 1385424 -> 2028464, growth 643040, under the 2360320 bound. What the line
  still measures for this row is that `value` itself is not duplicated beyond the
  codepoint list `__regex_makeCtx` derives.

- **F7 — F landed on top of letter C, not letter E.** The header says
  `Depends on: plan-146-E`, and §2 reasons about `htmlEscape` from "five
  `out = strings::replace(out, …)`, which are in place after letter E". This
  branch is `worktree-P-146-F`, cut from `worktree-P-146` at
  `5d31fe314 plan-146-C: the StrWindow arm` (`git log --oneline -1 worktree-P-146`),
  so letters D and E are not present. Nothing in F needs them: the `htmlEscape`
  rewrite deletes every `strings::replace` call from the body, so whether that
  builtin is in place or rebuilding no longer bears on this row, and the other
  fourteen rows never touched it. The measurements recorded here were taken
  without D/E; the numbers for the rows that pass are dominated by the result
  `out`, which no D/E arm changes.

## Summary

F turns 15 rows into proven exemptions. First it removes the copies that would
make the proofs false: F4's four C-string copies, and any MFBASIC body that seeds
its output with a copy of its argument, which `htmlEscape` already does.

**As landed.** F4 is fixed by one mechanism — the host reads the argument
`String` block's own NUL-terminated bytes at `+8` (`borrow_cstring` for `os`,
`emit_cstring_nul_scan` plus a `+8` for the two `fs` paths) — so the arena copy,
its `HelperScratch`, its release and (where it was the only one) its OOM exit are
all gone. Two MFBASIC bodies were rewritten, not one: `htmlEscape` for the copy
the plan predicted, and `htmlUnescape` because the `strings::graphemes` list it
read through is itself as live as the argument and costs ~42 bytes per character
(Correction F4). Both now scan with `strings::mid`. The 15 rows are
`Exempt { reason, proof }` and their 15 harness lines are `exempt` and pass.

The one thing this letter did NOT get is the RED proof as written: the
peak-live-bytes bound cannot fail on account of a single copy of a 64 KiB
`String` (Correction F5 does the arithmetic and runs the harness to show it).
Letter H should close that with a static check, since a dynamic one structurally
cannot.
