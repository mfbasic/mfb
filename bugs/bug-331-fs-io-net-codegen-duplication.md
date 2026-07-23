# bug-331: the fs/io/os/net codegen surface is ~18,400 lines of four parallel, hand-copied implementations of the same six emitters

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: In progress (partially landed 2026-07-23)
Regression Test: `scripts/artifact-gate.sh` + `scripts/test-accept.sh` — **byte-identical
generated output is the whole guarantee**; no new behavioral test, because a correct fix
changes no emitted byte.

## Progress (2026-07-23)

The structural groundwork this bug called for has **already landed** under bug-327 and
later cleanups: `src/target/shared/code/fs/` is now a real directory module
(`fs/{mod,paths,io,atomic}.rs`), `os.rs` became the `os/` directory module, and
`io_helpers.rs` was split into `io_stdin.rs`/`io_stdout.rs`/`io_terminal.rs`. Every line
number in the sections below is against the pre-split `b12213d2` worktree and is stale.

Landed this session, each its own commit and each gated byte-identical by
`scripts/artifact-gate.sh` (0 diffs, 1064 tests). `scripts/test-accept.sh` is green apart
from one pre-existing **flaky** fixture, `rt-behavior/resources/closed-default-tls-drop-rt`,
which SIGSEGVs (exit 139) on roughly one run in six and passes on retry (6/6 on re-run
here) — a TLS-drop-over-socket timing race, provably unrelated to this work.

**Done, byte-identical:**

- **§I** — deleted the shadowing `const EINTR_ERRNO` in `net/io.rs`/`net/poll.rs`.
- **§D** — the two hand-rolled `push_error_message_address` blocks in `fs/paths.rs` now
  call the helper (the io-file §D sites were already resolved by the io split).
- **§C** — `lower_fs_open_helper`'s eight unrolled `emit_branch_if_ascii_literal` calls
  folded into the `for`-loop form already used next door.
- **§B** — `lower_fs_write_text_path_helper`/`_bytes_` collapsed into
  `lower_fs_write_path_helper(bytes)`; `cap` allocated only in byte mode so the String
  path keeps its vreg numbering.
- **§H** — `emit_string_result_build` extracted, shared by net `read`/`receiveFrom` (the
  error tails were already shared via `emit_alloc`/`emit_fail`/`emit_call_validate_utf8`).
- **§J** — `emit_trailing_slash_trim` extracted (base_name/dir_name); the discarded
  `platform` param dropped from `lower_fs_path_join_helper`; `VALIDATE_UTF8_SYMBOL`/
  `SORT_STRING_LIST_SYMBOL` relocated to `codegen_utils.rs`.

**Remaining — deep investigation (2026-07-23) shows each needs a decision, not just typing:**

- **§A (marshal).** The full unification is NOT a byte-identical refactor. The fs sites are
  heterogeneous: the plain path marshal (`fs/paths.rs`), the `openFile`/`openFileWithin`
  variant that carries the embedded-NUL check *inside* the copy loop, and unrelated
  temp-name synthesis (`fs/atomic.rs`); their alloc/OOM prologues also differ (inline
  `ErrOutOfMemory`→`done` vs `os::marshal_cstring`'s branch-to-`alloc_fail`). The one part
  worth unifying — making every marshal reject embedded NUL — is the **Phase-6 behavior
  change** the doc already quarantines (`fs::exists("a\0b")` would change from `FALSE` to an
  error). Only the inner copy-and-terminate loop is byte-identically shareable, and even it
  spans sites with divergent register names; low value for the churn.
- **§G (dispatch collapse).** Collapsing the ~35 `fs.*`/`io.*` arms to the `net.*` shape is
  **not byte-identical** while the six `params: Vec::new()` arms exist: a single collapsed
  construction derives `params` from `spec.abi.params`, which would change the
  `.nplan`/`.nobj` descriptive model for those six. Per the doc's own Phase 1/5, that needs
  a deliberate per-arm verdict (a golden change) — which the Non-goals forbid folding into a
  refactor commit.
- **§F (UTF-8 decoder).** The two decoders have **drifted apart**: not a clean label infix
  but different label *names* (`read_second`/`read_third` vs `multi_start`/`line_read_third`)
  plus `LEN_OFFSET` vs `SEQ_LEN_OFFSET`, `BYTES_OFFSET` 8 vs 48, `got_len` vs `have_sequence`,
  and the `on_lf` (`trim_cr`) insertion. A byte-identical shared emitter needs ~15
  per-label/offset params over a ~195-line interleaved block — the indirection approaches
  the duplication it removes.
- **§E (buffered output).** The append pair is 40% different (63 substitutions/side): the
  buffer-block vregs are offset (`%v20`–`%v28` vs `%v30`–`%v39`) while the direct-write
  vregs (`%v40`/`%v41`) are shared, and the fd source differs in *instruction count*
  (immediate `1` vs a `FILE_OFFSET_FD` load). A sink-descriptor extraction is achievable but
  is a large mixed-shared/offset-vreg rewrite; the drain/`isBuffered` pairs the doc itself
  says "collapse … not for free."

Net: the remaining duplication is entangled with the two behavior/model changes the doc
quarantines (§A's NUL rejection, §G's six-arm verdict) or has drifted enough that a
zero-diff shared emitter is unwieldy (§E, §F). Per the doc's Non-goals ("if an item cannot
be landed with a zero-diff artifact gate, it must be dropped or re-scoped — not landed with
a golden regeneration"), these await an explicit scope decision.
- **§G** — the ~35 `fs.*`/`io.*` dispatch arms in `mod.rs` are still un-collapsed (44
  `Ok(CodeFunction {…})` constructions remain). **Hazard**: the six `params: Vec::new()`
  arms must be resolved by hand first.
- **§H** — the duplicated `net/io.rs` result builders.
- **§J** — `VALIDATE_UTF8_SYMBOL`/`SORT_STRING_LIST_SYMBOL`/`FS_PATH_JOIN_SYMBOL`
  placement in `fs/paths.rs` and the trailing-slash trim loops in `builder_fs_paths.rs`
  (low-value tidiness).

`src/target/shared/code/` contains four sibling implementations of the `fs`, `io`, `os`,
and `net` runtime helpers that were built at different times and never reconciled. They
independently open-code the same six emitters: the path→C-string marshal, the
allocation-failure error tail, the buffered-output append/drain/flag triple, the UTF-8
sequence decoder, the mode-string matcher, and the `CodeFunction` construction that ends
every dispatch arm. One of the four — `net/` — is a real directory module that has
already been refactored into the shape the other three should adopt: a shared
`emit_cstring`, a `lower_net_endpoint_helper` that collapses three near-duplicate
helpers behind two `bool` axes, and a single dispatch arm that constructs `CodeFunction`
once. The other three never adopted any of it.

The single correct end state a fix produces: **each of those six emitters is spelled
once**, in a `code/fs/` directory module that mirrors `code/net/`, and every `fs`/`io`
call site refers to it — with the generated binaries byte-identical to today's on every
target.

Nothing miscompiles today. This is filed because the duplication is already producing
drift, and the drift is silent: the same routine is written two different ways in two
files (see the mode matcher, §Current State C), the same constant is defined four times
under two spellings, and — the one item with observable consequences — **embedded-NUL
handling for paths already disagrees between `fs::openFile` and `fs::exists`** because
each hand-rolled its own marshal (see Open Decisions).

References:

- Found during the cleanup review, Agent 06 (`fs`/`io`/`os`/`net` codegen), findings
  #1–#8, #11, #12, #16, #17, #23 and INCIDENTAL (a).
- `src/docs/spec/stdlib/14_os.md:33` anchors `os.rs:marshal_cstring` as the named
  path-marshal seam — the spec already treats it as canonical; the fs files simply do
  not call it.
- bug-323 (helper-body 4-tuple / emitter preamble have no type alias) — **owns** Agent 06
  finding #23. The 4-tuple return type is spelled longhand 117 times tree-wide, 64 of
  them in the eight files this bug touches. Do not re-fix it here; land bug-323's alias
  first or after, but from one place.
- bug-322 (arena-alloc / internal-call / error-tail boilerplate, ~1,500 lines) — the
  allocation-failure prologue in §Current State A is a member of that cluster. This bug
  removes the fs/io instances *as a side effect* of extracting the marshal; bug-322 owns
  the tree-wide `CodeBuilder::emit_arena_alloc` seam.
- bug-321 (Linux backend triplication) — sibling structural-duplication bug in the
  per-target layer; disjoint file set.

## Current State

Measured against `b12213d2`, in the worktree `.claude/worktrees/cleanup-review`.

The surface, by line count (`wc -l`):

| File | Lines | Module shape |
| --- | --- | --- |
| `src/target/shared/code/fs_helpers.rs` | 153 | flat file, 2 fns, **declares no submodules** |
| `src/target/shared/code/fs_helpers_paths.rs` | 1,961 | flat sibling |
| `src/target/shared/code/fs_helpers_io.rs` | 2,841 | flat sibling |
| `src/target/shared/code/fs_helpers_atomic.rs` | 1,855 | flat sibling |
| `src/target/shared/code/io_helpers.rs` | 2,290 | flat file |
| `src/target/shared/code/builder_fs_paths.rs` | 676 | flat file |
| `src/target/shared/code/os.rs` | 2,116 | flat file |
| `src/target/shared/code/net/{mod,io,poll}.rs` | 869 + 1,876 + 255 | **directory module** |
| `src/target/shared/code/mod.rs` | 3,548 | dispatch |

`fs_helpers.rs` is a vestigial module *name*, not a module root: it is 153 lines holding
exactly two functions (`emit_errno_error_mapping:3`, `emit_fs_path_errno_error_mapping:58`)
and contains no `mod` declarations. The three `fs_helpers_*` files that read as its
children are flat siblings declared independently in `mod.rs:3079`, `:3081`, `:3083`,
`:3085`, and total **6,657 lines**. `net` next door is declared at `mod.rs:3136` and is a
real directory.

### A — the path→C-string marshal is open-coded 14 times; three canonical helpers exist and none is called from `fs`

Three helpers already implement this:

- `src/target/shared/code/os.rs:224-270` — `marshal_cstring` (vreg-parameterised; 5 callers,
  all inside `os.rs`: `:388`, `:525`, `:610`, `:620`, `:720`).
- `src/target/shared/code/net/mod.rs:81-118` — `emit_cstring` (stack-slot-parameterised;
  4 callers: `net/mod.rs:378`, `net/io.rs:831`, `:1105`, `:1681`).
- `src/target/shared/code/tls/mod.rs:236-273` — a **third** `emit_cstring`, byte-identical
  to `net/mod.rs`'s modulo the vreg numbers (`%v9`–`%v14` vs `%v17`–`%v22`); 8 callers in
  `tls/openssl.rs` and `tls/macos.rs`.

`grep -rn 'marshal_cstring\|emit_cstring' src/` returns **zero hits in any
`fs_helpers_*.rs` or `io_helpers.rs`**. Instead, the exact prologue
`add_immediate(return_register(), &len, 1)` → `move_immediate(ARG[1], "Integer", "1")` →
`branch_link(ARENA_ALLOC_SYMBOL)` → OOM tail → byte copy loop → `store_u8(ZERO, &dst, 0)`
is written out longhand at 14 sites:

- `fs_helpers_paths.rs:35`, `:166`, `:566`, `:709`, `:965`, `:1352`, `:1575`, `:1621` (8)
- `fs_helpers_io.rs:603`, `:1023` (2)
- `fs_helpers_atomic.rs:913`, `:1153`, `:1452`, `:1690` (4)

Each instance runs ~45 lines for the marshal proper and ~75 including its inline OOM
error tail, so the cluster is **~630–1,050 lines**. (A wider grep for the NUL-terminating
copy-loop tail — `store_u8(abi::ZERO, …)` — returns 29 hits across the three `fs_helpers_*`
files and 74 across all of `shared/code/`; the excess over 14 is the near-relatives:
join-buffer terminators, temp-name synthesis, and the `dir/file` truncation at
`fs_helpers_atomic.rs:687-697`. Those are variants of the same shape, not separate
concerns, and are in scope for the parameterised helper.)

**Verbatim duplicated pair** — `fs::exists` and `fs::isDirectory`/`isFile`, in the same
file, 130 lines apart. `fs_helpers_paths.rs:85-105`:

```rust
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&alloc, abi::RET[1]),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &alloc),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &alloc),
    ]);
```

and `fs_helpers_paths.rs:219-238` — identical instruction for instruction, differing only
in the one trailing `add_immediate(ARG[1], stack_pointer(), STAT_OFFSET)` that stages the
`stat` buffer. The 55 lines that precede each (`:31-79` and `:162-210`) are likewise
identical: the same alloc call, the same OOM check, and the same hand-rolled
error-message relocation pair (see D).

### B — the four `*_path_helper` functions in `fs_helpers_atomic.rs` are a two-axis matrix written out four times

`fs_helpers_atomic.rs` `lower_fs_write_text_path_helper:861`,
`lower_fs_read_text_path_helper:1101`, `lower_fs_write_bytes_path_helper:1400`,
`lower_fs_read_bytes_path_helper:1644` span lines **861–1855 = 995 lines** over two axes
(read/write × String/Bytes). Measured with `diff` over the exact function bodies:

| Pair | Body lines | Differing lines (both sides) |
| --- | --- | --- |
| `write_text` vs `write_bytes` | 240 / 244 | **16** |
| `read_text` vs `read_bytes` | 299 / 212 | 173 |
| `write_text` vs `read_text` | 240 / 299 | 195 |

The write pair is a near-verbatim copy: collapsing it behind a `bytes: bool` removes
~240 lines outright. The read pair diverges more (the text path validates UTF-8 and
builds a `String`; the bytes path builds a `List OF Byte`), so it collapses partially —
the shared open/`fstat`/read-loop/close prologue and the shared error tails, not the
whole body.

The fix shape already exists next door: `net/mod.rs:285`
`lower_net_endpoint_helper(symbol, platform_imports, platform, listen: bool, address: bool)`
is one function serving `net.connectTcp`, `net.connectTcpAddr`, and `net.listenTcp`.

### C — the same mode-string matcher is written two different ways, in the same file

`fs_helpers_io.rs:545` `lower_fs_open_helper` and `:929` `lower_fs_open_within_helper`
are ~180 duplicated lines. The clearest instance is the `"r"|"read"|"w"|…` matcher:

- `:656-727` — **72 lines**: eight hand-unrolled `emit_branch_if_ascii_literal(…)` calls,
  each seven lines, each differing only in the literal and the target label.
- `:1187-1206` — **20 lines**: a `for (lit, target) in [ … ]` loop over exactly the same
  eight `(literal, target)` pairs, calling `emit_branch_if_ascii_literal` once.

Adopting the loop form at `:656` alone removes **52 lines** with no output change (the
emission order is identical). The 15 lines that follow each matcher
(`:728-742` and `:1207-1221`, the `flags_done` ladder) are byte-identical.

The two helpers also share the C-string marshal (`:633-653` vs `:1039-1059`), which is
the same block as §A except that both here *do* carry the embedded-NUL rejection at
`:645-646` / `:1051-1052`.

### D — `push_error_message_address` is hand-rolled at 4 sites while the same file calls it correctly 16 times

The helper is `data_objects.rs:34-66`. Four sites re-implement its exact body — an `adrp`
+ `add_pageoff` pair on `RESULT_ERROR_MESSAGE_REGISTER` and the matching
`DataAddrHi`/`DataAddrLo` relocation pair — inline:

- `fs_helpers_paths.rs:52-79` (`ERR_ALLOCATION_SYMBOL`)
- `fs_helpers_paths.rs:183-210` (`ERR_ALLOCATION_SYMBOL`)
- `io_helpers.rs:468-495` (`ERR_OUTPUT_SYMBOL`)
- `io_helpers.rs:751-778` (`ERR_INPUT_SYMBOL`)

`io_helpers.rs` calls the real helper correctly at 16 other sites (`:563`, `:978`,
`:1231`, `:1238`, `:1250`, `:1622`, `:1629`, `:1641`, `:1653`, `:1665`, `:2199`, `:2211`,
`:2218`, `:2230`, `:2242`, `:2254`). ~112 lines; the substitution is mechanical and
provably output-preserving — the instruction and relocation sequences are identical.

### E — the buffered-output triple is implemented twice, once per sink

Six functions, two sinks, one routine each:

| Routine | stdout sink | file sink |
| --- | --- | --- |
| append | `io_helpers.rs:132-281` `emit_append_to_stdout_buffer` (150) | `fs_helpers_io.rs:283-432` `emit_append_to_file_buffer` (150) |
| drain | `io_helpers.rs:13-131` `lower_stdout_drain` (119) | `fs_helpers_io.rs:199-282` `lower_fs_file_drain` (84) |
| is/setBuffered | `io_helpers.rs:578-665` (89) | `fs_helpers_io.rs:435-504` (74) |

**666 lines total.** The append pair is the strongest case: `diff` reports 126 differing
lines out of 300, and every one of them is one of six mechanical substitutions —

1. sink state location: `load_u64("%v20", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET)` vs
   `load_u64("%v30", file, FILE_OFFSET_BUF_PTR)` (and the `_FILLED` / `_ENABLED` twins);
2. fd source: `move_immediate(return_register(), "Integer", "1")` vs
   `load_u64("%v31", file, FILE_OFFSET_FD)` + `move_register(return_register(), "%v31")`;
3. drain symbol: `STDOUT_DRAIN_SYMBOL` vs `FILE_DRAIN_SYMBOL`;
4. capacity const: `OUT_BUFFER_CAPACITY` vs `FILE_BUFFER_CAPACITY`;
5. label prefix: `_buf_` vs `_fbuf_`;
6. vreg block: `%v20`–`%v28` vs `%v30`–`%v39`.

The word-then-byte block copy (`:117-146` vs `:120-146`), the short-write retry loop, the
overflow-drain decision, and the oversize-chunk direct-write path are otherwise
instruction-for-instruction identical — including the bug-51 comments, which have merely
been reworded. This is a textbook sink-descriptor parameterisation.

The drain and is/setBuffered pairs are structurally parallel but not verbatim (`diff`:
95/203 and 89/163 differing lines) — the stdout side carries an `app_mode` early-out and
a `finalize_vreg_body_with_locals(FRAME_SIZE)` the file side does not. They collapse
behind the same descriptor with an `Option`-typed app-mode arm, not for free.

### F — `readChar` and `readLine` share the entire UTF-8 sequence decoder, with exactly one semantic delta

`io_helpers.rs:1317` `lower_io_read_char_helper` and `:1685` `lower_io_read_line_helper`
each contain a full 1/2/3/4-byte UTF-8 lead-byte decoder with the overlong- and
surrogate-rejection special cases (`_three_not_e0`, `_three_general`, `_four_not_f0`,
`_four_general`) at `:1404-1586` and `:1871-2055` — **183 and 185 lines**.

Diffed after normalising the `_char_`/`_line_` label infix, the two are identical except:

1. **the one real delta** — `readLine` inserts two instructions after the byte read:
   `compare_immediate("%v10", "10")` / `branch_eq(&trim_cr)`, the LF terminator check;
2. the stack-slot const name (`LEN_OFFSET` vs `SEQ_LEN_OFFSET`);
3. the continuation label name (`got_len` vs `have_sequence`, `eof` vs
   `{symbol}_read_eof`);
4. `readChar` binds `read_second`/`read_third`/`read_fourth` to locals while `readLine`
   spells the same strings inline as `format!`.

Items 2–4 are naming, not behavior. The index's "four real deltas" **overstates it**: there
is one. Extracting `emit_utf8_sequence_read(…, on_lf: Option<&str>, len_slot, continue_label)`
removes ~180 lines and makes the decoder a single place to fix.

### G — 35 `fs.*`/`io.*` dispatch arms repeat the same 20-line construction; the `net.*` arm beside them already shows the fix

`mod.rs` `lower_runtime_helper` contains **45** `Ok(CodeFunction { … })` constructions.
**35** of them are `fs.*`/`io.*` arms, spanning `mod.rs:1489-2400` (**912 lines**). Each
arm is: compute a couple of `bool`s, call one lowering function, then re-spell

```rust
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec.abi.params.iter().map(|param| CodeParam { … }).collect(),
                returns: spec.abi.returns.to_string(),
                frame, stack_slots, instructions, relocations,
            })
```

`mod.rs:2402-2483` — the `net.*` arm — is the correct shape and sits directly below the
last `fs.*` arm: **one** guard (`call if call.starts_with("net.")`), an inner `match` over
20 calls yielding the 4-tuple, an `other =>` error arm, and **one** construction at
`:2464-2482`. Converting the `fs.*`/`io.*` arms to that shape removes ~600 of the 912
lines.

**Hazard, recorded before anyone collapses this:** the arms are not uniform. Six of the 45
constructions in the range pass `params: Vec::new()` where the other 39 derive `params`
from `spec.abi.params` — e.g. `mod.rs:1558` (`io.flush`) hardcodes `Vec::new()` while
`mod.rs:1524` (`io.print`) maps `spec.abi.params`. That is the latent drift Agent 04
flagged (its finding #1, same code, whole-function scope). A naive collapse silently
picks one and changes the `.nplan`/`.nobj` descriptive model for the other. **Resolve the
six by hand, with a golden diff, before collapsing.**

### H — `net/io.rs` error tails and result builders duplicated

`net/io.rs` contains two result-builder blocks that are **byte-identical** — `diff` over
`:425-456` and `:1397-1428` (the arena `String` build-and-return, 32 lines) reports *zero*
differing lines. The `List OF Byte` builder at `:473-527` vs `:1431-1480` differs by 9
lines (an extra `build_list` label, a `STR_OFFSET` spill, and a three-line OK-return tail
that one arm inlines). Together with the file's repeated alloc-fail / closed-handle /
timeout error tails and epilogues, this is ~230 lines. These belong to the bug-322
error-tail cluster; extracting them into `net/` local emitters is the local half.

### I — `EINTR_ERRNO` is defined four times, and one of the four is a shadow of an in-scope parent

```
src/target/shared/code/net/mod.rs:30        const EINTR_ERRNO: &str = "4";
src/target/shared/code/net/io.rs:15         const EINTR_ERRNO: &str = "4";
src/target/shared/code/net/poll.rs:12       const EINTR_ERRNO: &str = "4";
src/target/shared/code/fs_helpers_io.rs:6   const EINTR_ERRNO: &str = "4";
```

plus two aliases of the same value elsewhere (`stdin_broadcast.rs:19`
`STDIN_EINTR_ERRNO`, `audio/alsa.rs:28` `EINTR`). **`net/io.rs:10` and `net/poll.rs:7` both
do `use super::*`**, so `net/mod.rs:30` is already in scope in both; the local `const` at
`net/io.rs:15` and `net/poll.rs:12` shadows the glob import (glob imports lose to local
items, so this compiles silently). Deleting those two lines is a no-op today and removes
the possibility of the three copies disagreeing.

### J — `builder_fs_paths.rs` vs `fs_helpers_paths.rs`: a principled split with names that hide it, and a leaking boundary

The real split is **inline vs. out-of-line**, not two flavours of "paths":

- `builder_fs_paths.rs` (676) is `impl CodeBuilder` — inline lowering for the five pure
  string operations `fs.pathJoin`/`pathBaseName`/`pathDirName`/`pathExtension`/
  `pathNormalize` (`:4-18` dispatch). No syscalls.
- `fs_helpers_paths.rs` (1,961) emits standalone runtime `CodeFunction`s that call
  `platform.emit_path_exists`/`emit_path_stat`/etc. Syscall-bearing.

The names convey neither. And the boundary leaks in both directions:

- `fs_helpers_paths.rs:1796` `FS_PATH_JOIN_SYMBOL` and `:1806`
  `lower_fs_path_join_helper` are **pure string joining** in the syscall file — the
  function's only external call is `ARENA_ALLOC_SYMBOL`, and it takes a
  `platform: &dyn CodegenPlatform` it immediately discards (`:1811`, `let _ = platform;`).
  Its sole caller is `mod.rs:1211`, and the inline `lower_fs_path_join` in
  `builder_fs_paths.rs:28` just calls it.
- `fs_helpers_paths.rs:1790` `VALIDATE_UTF8_SYMBOL` and `:1793` `SORT_STRING_LIST_SYMBOL`
  are generic string/list symbols with nothing path-related about them; their consumers
  are in `codegen_utils.rs:9`, `:130`, `:142`, `:145`, `:163`.

Related: the trailing-slash trim loop in `builder_fs_paths.rs` is written three times —
`:88-97` and `:170-179` are byte-identical (12 lines each), and `:256-265` differs only in
its guard (`compare_immediate(&length, "0")` / `branch_eq(&empty)` vs
`compare_immediate(&length, "1")` / `branch_le(&trim_done)`). The backward scan that
follows each repeats similarly. ~55 lines.

## Root Cause

Four teams, four eras, no shared module. `net/` was refactored into a directory with
shared emitters (`net/mod.rs:45` `emit_alloc`, `:61` `emit_fail`, `:81` `emit_cstring`,
`:285` the two-axis endpoint helper) and a collapsed dispatch arm. `os.rs` grew its own
`marshal_cstring` (`:224`) and got it spec-anchored at
`src/docs/spec/stdlib/14_os.md:33`. `tls/` copied `net/`'s `emit_cstring` verbatim into
`tls/mod.rs:236` rather than importing it. `fs_helpers_*` and `io_helpers.rs` — the oldest
and largest of the four, at 8,947 lines combined — adopted none of it and instead grew by
copy-paste from their own nearest neighbour, which is why the duplicates cluster
*within* files (§A's pair is 130 lines apart in one file; §C's two mode matchers are 530
lines apart in one file).

Because `fs_helpers.rs` was never turned into a directory root, there was no obvious place
to put a shared fs emitter, so each new helper re-derived one. The vestigial name is not
cosmetic — it is the structural reason the duplication had nowhere to collapse into.

## Goal

- Every emitter in §A–§J is spelled once and called from every site that needs it.
- `src/target/shared/code/fs/` exists as a directory module mirroring `code/net/`, owning
  the shared fs emitters; `fs_helpers.rs`'s two functions move into it and the vestigial
  file is gone.
- The `fs.*`/`io.*` dispatch arms in `mod.rs` have the `net.*` arm's shape: one guard, one
  inner match yielding the 4-tuple, one `CodeFunction` construction.
- **`scripts/artifact-gate.sh` reports zero byte differences** in generated code for every
  target after every phase except the two that are explicitly allowed to shift (see
  Non-goals).
- `scripts/test-accept.sh` is green with zero golden churn.

### Non-goals (must NOT change)

- **Any emitted byte.** This is a pure refactor. Every phase below is gated on
  `artifact-gate.sh` reporting a byte-identical image. The two exceptions are called out
  explicitly and must be landed as separate, individually-reviewed commits with a
  regenerated golden and a diff confirming the delta is exactly the intended one:
  1. the embedded-NUL unification (see Open Decisions) — a real behavior change;
  2. any dead-label or branch-to-next-instruction removal, which shifts instruction
     bytes. **Do not bundle either into a §A–§J extraction commit.**
- The `.mfp` wire format, the `File` record layout (`FILE_OFFSET_*`), the arena state
  offsets (`ARENA_OUT_*`), and the runtime symbol names.
- The `RuntimeHelperSpec` / `spec.abi` model. In particular, the six arms that pass
  `params: Vec::new()` must keep whatever they emit today until someone decides,
  deliberately and with a golden diff, that they were wrong (§G).
- The 4-tuple return type — **bug-323 owns it.** Do not introduce a competing alias here.
- The tempting wrong fix, forbidden explicitly: *unifying `readChar`/`readLine` by making
  `readChar` also stop at LF*, or *unifying the marshals by dropping the NUL check from
  `openFile` because more call sites omit it than have it*. Both silently change behavior
  to make a refactor tidier. Majority-of-call-sites is not an argument about correctness.

## Blast Radius

Found by search, not memory. Every site classified.

**Fixed by this bug:**

- `fs_helpers_paths.rs:35,166,566,709,965,1352,1575,1621` — open-coded marshal (§A).
- `fs_helpers_io.rs:603,1023` — open-coded marshal, both *with* the NUL check (§A/§C).
- `fs_helpers_atomic.rs:913,1153,1452,1690` — open-coded marshal (§A).
- `fs_helpers_atomic.rs:861,1101,1400,1644` — the four `*_path_helper`s (§B).
- `fs_helpers_io.rs:545,929` — the open/open-within pair (§C).
- `fs_helpers_paths.rs:52-79,183-210`; `io_helpers.rs:468-495,751-778` — hand-rolled
  `push_error_message_address` (§D).
- `io_helpers.rs:13,132,578,623`; `fs_helpers_io.rs:199,283,435,468` — the buffered-output
  triple (§E).
- `io_helpers.rs:1404-1586,1871-2055` — the UTF-8 decoder (§F).
- `mod.rs:1489-2400` — 35 `fs.*`/`io.*` dispatch arms (§G).
- `net/io.rs:425-456,1397-1428,473-527,1431-1480` — the duplicated result builders (§H).
- `net/io.rs:15`, `net/poll.rs:12` — shadowing `EINTR_ERRNO` (§I).
- `fs_helpers_paths.rs:1790,1793,1796,1806`; `builder_fs_paths.rs:88-97,170-179,256-265` —
  boundary leaks and trim loops (§J).
- `fs_helpers.rs:1-153` — dissolved into `code/fs/` (§ the table above).

**Latent, same hazard, in scope only for the shared helper — not rewritten here:**

- `tls/mod.rs:236-273` — third `emit_cstring`, byte-identical to `net/mod.rs:81-118` modulo
  vreg numbers. Should call the shared marshal once it exists, but `tls/` has its own
  cleanup bug (bug-317 touches its error paths) and 8 call sites; convert it in a
  follow-up so a `tls` regression cannot be confused with an `fs` one.
- `os.rs:224-270` `marshal_cstring` — becomes *the* shared helper (it is already the
  spec-anchored one), so `os.rs`'s five call sites change only by import path.
- `stdin_broadcast.rs:19`, `audio/alsa.rs:28` — `EINTR` under other names. Out of scope:
  different subsystems, and Agent 05 finding #11 / bug-322 territory.

**Unaffected:**

- `crypto.rs`, `crypto_ec*.rs`, `link_thunk.rs`, `float_format.rs` — their
  `store_u8(abi::ZERO, …)` hits are not path marshals (key/DER/thunk buffers).
- Every `src/target/{macos,linux}_*/` backend — this bug touches only `shared/code/`; the
  `platform.emit_path_*` hooks keep their exact signatures.
- `builder_fs_paths.rs:377,667` — `store_u8(ZERO, …, 8)` writes a String header byte, not a
  C-string terminator.

## Fix Design

Create `src/target/shared/code/fs/` as a directory module mirroring `code/net/`:

```
code/fs/
  mod.rs          — shared emitters + submodule decls AT THE TOP (see Rejected, below)
  cstring.rs      — the one path marshal
  buffered_output.rs — the sink-descriptor-parameterised append/drain/flag triple
  paths.rs        — was fs_helpers_paths.rs
  io.rs           — was fs_helpers_io.rs
  atomic.rs       — was fs_helpers_atomic.rs
  errno.rs        — was fs_helpers.rs (both fns)
```

The five pieces, and where the risk actually is:

1. **`marshal_cstring` (shared).** Promote `os.rs:224-270` — it is already the
   spec-anchored form and already vreg-parameterised, so it composes with the fs helpers'
   vreg discipline; `net/mod.rs:81-118`'s stack-slot form does not. Give it a
   `reject_embedded_nul: bool` (see Open Decisions) and a `uniq` label seed. **Risk
   concentrates here**: the 14 fs sites differ in vreg *numbering*, and vreg numbering
   feeds the linear-scan allocator, so a naive extraction can shift spill decisions and
   therefore bytes. Land §A one call site at a time, gating each on `artifact-gate.sh`.
2. **`buffered_output.rs` parameterised by a sink descriptor.** A plain struct — not a
   trait — carrying: state base (`ARENA_STATE_REGISTER` + `ARENA_OUT_*` offsets, or a
   `File*` vreg + `FILE_OFFSET_BUF_*`), fd source (immediate `"1"` or a load), drain
   symbol, capacity const, label infix, and vreg base. A struct keeps the whole thing a
   compile-time value and avoids adding a dyn-dispatch seam to a code path that has none.
3. **`emit_utf8_sequence_read`.** Signature
   `(instructions, symbol, uniq, len_slot: usize, continue_label: &str, on_lf: Option<&str>, …)`.
   The `on_lf` is the single real delta from §F. Lowest-risk item in the bug: one
   extraction, two call sites, zero cross-file coupling.
4. **`mod.rs` dispatch conversion.** Copy the `net.*` arm's shape verbatim. Do the six
   `params: Vec::new()` arms first, by hand, each with its own golden diff (§G hazard).
5. **The `net/` half (§H, §I) and the naming/leak fixes (§J)** are independent and can land
   in any order.

Rejected alternatives, so nobody re-litigates them:

- *A `trait FsSink` instead of a descriptor struct.* Rejected: introduces dynamic dispatch
  into an emitter with two implementations known at compile time, for no gain, and the
  tree already has one god-trait problem (`types.rs`'s 65-method `CodegenPlatform`).
- *Keep the three `fs_helpers_*` files flat and just add a `fs_shared.rs`.* Rejected: this
  is exactly how the current state arose. The vestigial `fs_helpers.rs` is the structural
  cause (§Root Cause); leaving it means the next helper copies rather than imports.
- *Collapse all four `*_path_helper`s into one function with two `bool`s.* Rejected on the
  read axis: at 173 differing lines the read pair does not merge cleanly, and forcing it
  produces a function with unrelated branches interleaved. Collapse the write pair fully
  (2 `bool`s → 1), and factor only the shared prologue/tails out of the read pair.
- *Sort the `fs.*`/`io.*` arms alphabetically while converting them.* Rejected: reordering
  match arms can reorder emitted `CodeFunction`s and therefore the object layout. Preserve
  order exactly; that is what makes the artifact gate meaningful.

Expected output shift: **none**, in every phase but the two named in Non-goals.

## Phases

Ranked by value ÷ risk. **Phase 2 lands first** — it is the highest-confidence,
zero-judgement work and it establishes that the artifact gate is actually catching things
before any judgement-bearing extraction runs.

### Phase 1 — audit + gate baseline (no code change)

- [ ] Record a clean `scripts/artifact-gate.sh` baseline image hash for every target.
- [ ] Enumerate the six `params: Vec::new()` dispatch arms (§G) and write each one's
      verdict — intentional or drift — into this file. This gates Phase 5.
- [ ] Confirm the §A site list is complete by re-running the marshal-prologue grep and the
      `store_u8(abi::ZERO, …)` grep, and classify each of the 29 fs-file NUL-store hits as
      marshal / variant / unrelated.

Acceptance: baseline hashes recorded; every §A site and every §G arm has a written verdict.
Commit: —

### Phase 2 — mechanical, provably output-preserving substitutions (lands first)

Highest value per unit of risk: ~220 lines, no judgement calls, and each change is
independently verifiable by inspection.

- [ ] §D — replace the 4 hand-rolled `push_error_message_address` blocks with calls.
- [ ] §C — replace the 8 unrolled `emit_branch_if_ascii_literal` calls at
      `fs_helpers_io.rs:656-727` with the `for` loop already present at `:1187-1206`.
- [ ] §I — delete `net/io.rs:15` and `net/poll.rs:12`; confirm both resolve to
      `net/mod.rs:30` via their existing `use super::*`.
- [ ] §J — move `VALIDATE_UTF8_SYMBOL` / `SORT_STRING_LIST_SYMBOL` out of
      `fs_helpers_paths.rs:1790,1793` to `codegen_utils.rs` (import-path change only).

Acceptance: `artifact-gate.sh` byte-identical on every target; `test-accept.sh` green,
zero golden churn.
Commit: —

### Phase 3 — the `code/fs/` directory (pure moves)

- [ ] Create `code/fs/`; move `fs_helpers.rs` → `fs/errno.rs`, `fs_helpers_paths.rs` →
      `fs/paths.rs`, `fs_helpers_io.rs` → `fs/io.rs`, `fs_helpers_atomic.rs` →
      `fs/atomic.rs`. Declare submodules at the **top** of `fs/mod.rs` (`net/mod.rs:865`
      declares its submodules at line 865 of 869 — do not copy that).
- [ ] Move `FS_PATH_JOIN_SYMBOL` + `lower_fs_path_join_helper` (§J) to `fs/paths_pure.rs`
      or fold into `builder_fs_paths.rs`; drop the discarded `platform` param.
- [ ] Update `mod.rs:3079-3085` declarations and all `use` paths.

Acceptance: no content change beyond `use`/`mod` lines; `artifact-gate.sh` byte-identical.
Commit: —

### Phase 4 — the extractions

Each bullet is its own commit, each gated on `artifact-gate.sh`.

- [ ] §F — `emit_utf8_sequence_read` into `io_helpers.rs`; two call sites. (Do this first:
      one extraction, one file, and the `on_lf` parameter makes the sole real delta
      explicit.)
- [ ] §E — `fs/buffered_output.rs` with the sink descriptor; the append pair first, then
      drain, then is/setBuffered.
- [ ] §A — the shared `marshal_cstring` in `fs/cstring.rs`, promoted from `os.rs:224`.
      **One call site per commit** — vreg-numbering shifts can move spill decisions.
- [ ] §B — collapse `write_text`/`write_bytes` behind `bytes: bool`; factor only the shared
      prologue and error tails out of the read pair.
- [ ] §H — extract the `net/io.rs` `String`/`List` result builders and the repeated error
      tails.

Acceptance: `artifact-gate.sh` byte-identical after **each** commit; `test-accept.sh`
green.
Commit: —

### Phase 5 — the `fs.*`/`io.*` dispatch collapse

- [ ] Resolve the six `params: Vec::new()` arms per the Phase 1 verdicts, individually,
      each with its own golden diff.
- [ ] Convert `mod.rs:1489-2400` to the `net.*` shape (`mod.rs:2402-2483`), preserving arm
      order exactly.

Acceptance: `artifact-gate.sh` byte-identical; the `.nplan`/`.nobj` goldens show either no
delta or exactly the deltas approved in the Phase 1 verdicts.
Commit: —

### Phase 6 — the behavior change (separate, deliberate)

- [ ] Land the embedded-NUL decision from Open Decisions as its own commit, with a
      regression test per policy option and a regenerated golden. **This is the only phase
      permitted to change behavior.**

Acceptance: the new NUL test passes; every other test unchanged; the golden delta is
exactly the intended one.
Commit: —

## Validation Plan

- Regression test: none for Phases 2–5 — the byte-identical artifact is the test. For
  Phase 6, an rt-behavior test asserting the chosen embedded-NUL behavior for
  `fs::exists`, `fs::openFile`, `os::getEnv`, and a `net` host argument.
- Runtime proof: `scripts/artifact-gate.sh` (execution-free, ~5 min) after every commit —
  this is the gate that makes a 3,000-line refactor safe; `scripts/test-accept.sh` at the
  end of each phase.
- Doc sync: `src/docs/spec/stdlib/14_os.md:33` anchors `os.rs:marshal_cstring` and must be
  re-anchored when the function moves to `code/fs/cstring.rs`. Re-check the spec anchor
  sweep — moving four files will invalidate any raw-line anchors into them.
- Full suite: `scripts/test-accept.sh` plus `cargo fmt` (note: the tree needs a second
  `cargo fmt` pass in `repository/`, which is not a workspace member).

## Open Decisions

- **Embedded-NUL rejection in the unified marshal — recommend making the check the
  default (`reject_embedded_nul: true`), with opt-out only where a golden proves it
  changes bytes.** Today the four implementations disagree, and unifying them *will* make
  one behavior win. Verified current state:
  - `fs::openFile` **rejects**: `fs_helpers_io.rs:645-646` emits
    `compare_immediate(&byte, "0")` / `branch_eq(&invalid)` inside the copy loop, so
    `openFile("a\0b", …)` raises `ErrInvalidArgument`. `openFileWithin` does the same at
    `:1051-1052`.
  - `fs::exists` **silently truncates**: `fs_helpers_paths.rs:96-97` is the same two lines
    of the same loop **without** the compare — the `\0` is copied into the buffer and the
    C boundary truncates there, so `fs::exists("a\0b")` answers a question about `"a"`.
    All 8 marshal sites in `fs_helpers_paths.rs` and all 4 in `fs_helpers_atomic.rs` omit
    the check.
  - `os.rs:224-270` `marshal_cstring` omits it; `net/mod.rs:81-118` `emit_cstring` omits it
    (its doc comment at `:77` merely *asserts* "NUL-free" without checking);
    `tls/mod.rs:236` omits it.

  So today the answer depends on which function you called. Rejecting is the
  security-correct default: silent truncation means a path check and the subsequent
  operation can disagree about which file is named, which is a confused-deputy shape. It
  is nonetheless **a real behavior change** — a program that today gets `FALSE` from
  `fs::exists("a\0b")` would get an error instead — and must be landed as Phase 6, on its
  own, with a test and a regenerated golden. It must not ride along inside a §A
  extraction commit.
- **The six `params: Vec::new()` dispatch arms (§G)** — resolve individually in Phase 1 vs.
  collapse and accept whichever value wins. Recommend individual resolution: the two
  behaviors produce different `.nplan`/`.nobj` output, and Agent 04 flagged the same six as
  possible latent drift, so this is the moment to decide rather than freeze an accident.
- **`tls/mod.rs:236`'s third `emit_cstring`** — convert in this bug vs. a follow-up.
  Recommend follow-up: `tls/` has 8 call sites and its own open bug (bug-317), and keeping
  it out means an `fs` byte-diff cannot be mistaken for a `tls` one.

## Summary

Real engineering risk is in exactly two places, and neither is the volume of lines. First,
**§A's vreg renumbering**: the fs marshal sites differ in which vregs they use, vreg
numbering feeds the linear-scan allocator, and a shifted spill decision changes bytes —
hence one call site per commit, each gated on `artifact-gate.sh`. Second, **§G's six
`params: Vec::new()` arms**: they are the one place where the arms are genuinely not
uniform, and a naive collapse would silently pick a winner and change the descriptive
model. Everything else — §D, §C's mode matcher, §I, §F's decoder — is mechanical and
provably output-preserving, which is why Phase 2 lands first: it proves the gate works
before any judgement-bearing change runs.

Left untouched: the 4-tuple return type (**bug-323**), the tree-wide arena-alloc and
error-tail seams (**bug-322**), `tls/`'s third marshal copy, every per-target backend, and
the `File`/arena record layouts. The embedded-NUL unification is quarantined into its own
phase because it is the one item here that changes what a compiled program does.
