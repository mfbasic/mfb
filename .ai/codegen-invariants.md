# Codegen & IR invariants

Load-bearing facts about the MFB compiler's native code generation and IR lowering (aarch64 / riscv64 / x86-64 / macOS / Windows). Each section captures one invariant: the mechanism, the failure it causes, and the fix.

## Owned-value drops must free-and-null the cleanup slot (loop double-free)

`emit_owned_value_drop` / `emit_closure_drop` (`src/target/shared/code/builder_owned_cleanup.rs`) null-guard the cleanup slot — `if slot != 0 { arena_free(slot) }` — and the guard is sound ONLY if the slot reads 0 on every path that reaches the drop without a store. The slot is zero-initialized once at the prologue (`function_lowering.rs`, the `owned_value_slots` splice). That one-time init is NOT enough across a loop back-edge: a loop-scoped owned temp whose initializer is short-circuit-evaluated (e.g. a **record-returning call in a `WHILE` condition** that the last iteration skips) leaves the slot holding the *previous* iteration's already-freed pointer, so the drop frees it AGAIN. A non-immediate double-free (other allocs intervened, so `arena.rs`'s immediate-double-free idempotency guard misses it) corrupts the free-list and any live block that reused the freed one. Fix: zero the slot right after `arena_free` (free-and-null), so a re-reached drop reads 0 and skips (bug-440).

The runtime symptom is nondeterministic — `arena_free` entropy-scrubs the freed block, so a use-after-free of a corrupted sibling reads random bytes; the corruption also depends on whether the double-freed block was reused live (allocation-order/tty-dependent). So a black-box rt test is UNSOUND here (0/20 under a pipe even unfixed) — assert the fix in the `.ncode` instead (`tests/codegen_owned_drop_free_and_null.rs`: every `owned_value_free_skip*` cleanup ends with a `str xzr` to its slot, not a bare `bl _mfb_arena_free`). The free-and-null store uses only `abi::ZERO`/`stack_pointer` (no vreg alloc), so its `.ncode` delta is purely additive zero-stores — it does NOT shift vreg numbering, and it is byte-neutral for fixtures whose owned drops are never re-reached only in that the store is still emitted (present, not eliminated).

## `_mfb_*` runtime-helper calls clobber all caller-saved integer registers

`_mfb_arena_alloc` (`lower_arena_alloc` in `src/target/shared/code/entry_and_arena.rs`) is a vreg-allocated, PCS-framed helper: all caller-saved integer registers (x0–x17) are clobbered; callee-saved (x19–x28) are preserved by its frame. The historical survivor set (x8/x11/x12/x13/x17) was byte-identical-migration scaffolding and was deleted after a tree-wide audit proved no caller depended on it — do NOT rely on any register surviving the call.

Canonical bug shape: the MUT-append-grow segfault (`tests/regression-mut-append-grow-rt`). `lower_list_insert_collection` computed the new list's data length in x15 before the alloc, then read x15 after it — but the grow path had clobbered x15 with a pointer. That poisoned DATA_LENGTH; the next append read the pointer as a huge size → runaway grow → mmap ENOMEM → SIGSEGV at element 17. Fix: spill to a stack slot before the `bl`, reload after. Same pattern fixed in `lower_list_remove_at` (x15), `lower_map_concat` (x14), `lower_map_remove_key` (x14/x15).

Rule of thumb: anything needed AFTER a `bl _mfb_*` runtime-helper call must live in a stack slot (or a vreg — the regalloc clobber model treats `_mfb_*` calls as destroying ALL integer registers, so vregs spill automatically).

## Records inline their String fields (offset, not pointer)

A record's `String` field is not a pointer. The word at `8*i` is the offset, relative to the record's own block, of an inlined `{len, bytes, NUL}` sub-block in a trailing data region. Blocks sit contiguously, each 8-aligned, each `len + 9` bytes. Block size = `8*n + Σ align8(len+9)`.

Only `Address`, `Datagram`, `DatagramText`, `AudioDevice` keep pointer strings — every other record inlines. See `record_field_is_inlined` (`src/target/shared/code/builder_collection_layout.rs:586`); the authoritative construction is `emit_build_inlined_record` in the same file. Mirror it; never hand-roll a record from a mental model.

Why it bites: the caller sizes/copies a record by walking the data region contiguously (`emit_record_block_size_to_slot` ignores the stored offsets when sizing), so hand-built code that stores a pointer and omits the region makes the caller read garbage as a length and add 9 → `ldr x11,[x10]; add x11,x11,#9`. Huge garbage → "Allocation failed" (7-701-0001); small-but-wrong → SIGSEGV at a different address every run. A scalar-only record hides this completely — `8*n` is then exactly right, so scalar tests passing proves nothing about the String path.

Note `8*i` means records can never be C structs — `SF_INFO.channels` lands at 12 in C, 16 in a record.

## vreg allocation order is load-bearing (don't delete a dead `temporary_vreg()`)

Deleting dead code in a native codegen builder (`src/target/shared/code/*`) is often NOT byte-identity-neutral, even when the removed value is truly unread.

`temporary_vreg()` / `allocate_register()` have a side effect: each call advances `next_vreg` and pushes a `vreg_eager` placeholder. So the order and count of vreg allocations determines the emitted register numbers — a documented byte-identity invariant (see the `emit_executable_path_into` doc comment: "invoke this FIRST … so it keeps the exact vreg-allocation order — and therefore the byte-identical output"). Removing an unused vreg allocation shifts `next_vreg` for every subsequent allocation in that function; if the function runs for many fixtures (e.g. `lower_value_inner` runs for every value lowering), it churns essentially ALL `.ncodesum` goldens for a cosmetic gain.

How to apply: when a dead-code removal orphans a scratch vreg, KEEP the allocation to preserve numbering and drop only the now-unused name binding — `let _ = self.temporary_vreg();` with a load-bearing comment (the allocation's side effect is the point). Then PROVE byte-identity by running the full `scripts/artifact-gate.sh` and confirming `0 diff(s)` — a real change would surface as a DIFF on some unrelated fixture. For a removal you intend to shift codegen (e.g. deleting an emitted instruction), do a before/after `.ncode` diff (git-free: revert the one line, rebuild, dump, diff, re-remove) to prove the delta is ONLY the intended op before regenerating the `.ncodesum`. See the artifact-gate and byte-identity notes.

## Param-passthrough return borrow (elide the deep-copy)

A function whose EVERY value-return is a bare parameter (`FUNC copyStrs(xs) RETURN xs`, the identity/passthrough shape) does not need to deep-copy on return. The copy can be MOVED to the caller's ownership boundary with byte-identical value semantics:

- Callee (`lower_returned_value`): skip the `copy_flat_block` and return the parameter pointer directly — a BORROW of the caller's argument block.
- Caller (`value_needs_owning_copy`): classify the call result as an aliasing source. Then the EXISTING discipline does the rest for free — `register_pending_temp` already early-returns on `value_needs_owning_copy` (so a borrowed result is never freed as a temp), and `lower_value_owned` already deep-copies an aliasing source at any owning store (LET/assign/return/global). So a read-only-and-discarded result (`len(copyStrs(base))`) pays NO copy; a result stored into an owned binding is copied exactly once at the store.

Both sides MUST key off the SAME predicate (`function_returns_param_borrow`, computed from the `functions` map available in the builder) or they desync: callee-skips-but-caller-frees = double-free; callee-copies-but-caller-treats-as-borrow = leak.

Load-bearing gotchas:
- A parameter carries NO `OwnedValue` cleanup — that absence is what marks it a borrow of the caller's block, NOT the `by_ref` flag (params are inserted with `by_ref: false` in `lower_function`). Gate the callee skip on "the returned local owns no `OwnedValue` cleanup at its stack slot": it is both the exact soundness condition AND fail-safe (a false-positive falls through to a copy — at worst a missed elision, never a use-after-free). This is the same ownership test `plan_returned_move` (the owned-LOCAL move-elision) uses.
- Conservative predicate: require >=1 value-return, EVERY return a bare param `Local`, and NO returned param reassigned (`collect_reassigned_locals`) or address-taken (`collect_address_taken_locals`) — a mutated/aliased param is no longer the caller's untouched arg. Use the exhaustive `NirVisitor` seam so a new return/mutation route can't slip the gate.
- The elision is INVISIBLE to `.ir`/`.ast` (a codegen-only decision) but churns `.ncode` for every fixture containing a passthrough function; a fresh-returning function (`RETURN append(xs,…)`, a `Call` not a bare param) is correctly NOT marked.
- Measured: the benchmark `list (Dynamic) copy` pattern (`len(copyStrs(base))` over a 1000-element String list) went from a full-list deep-copy per call to a pointer return — a ~2900x drop on the isolated micro-bench. Related: collection memory management and the read-only element-borrow twin (get-borrow pending-temp + MATCH-desugar).

## Producer-side `Operand::imm` is an allocation trap

Replacing `.field("imm"/"offset", &n.to_string())` (builds `Operand::Raw`) with a typed `Operand::imm(n)` at the PRODUCER is not automatically an allocation win, and is often a net LOSS. The trap: `Operand::rendered()` (operand.rs) returns `Cow::Borrowed` for `Raw`/`Phys` (0 alloc) but `Cow::Owned(render())` for `Imm`/`VReg` — so `Imm::rendered()` allocates a fresh `String` every call.

`Raw` costs 2 allocs at construction (`to_string()` + the `Box<str>`) then renders for free; `Imm` costs 0 at construction then 1 alloc per `.rendered()`. So `Imm` only wins when the field is rendered ≤1 time downstream.

The `imm`/`offset` fields of spill/reload and stack-access instructions are the worst case: `finalize_frame` (codegen_utils.rs) `rendered()`s them multiple times (offset resolution passes) and then overwrites the field with `Operand::imm(resolved_offset)` anyway — so the producer's value only ever serves transient reads. Making it `Imm` there just turns cheap borrows into repeated allocations.

Measured: converting all ~20 producer sites in `abi.rs` + x86/riscv `regmodel` raised `total_allocs` on `mfb test tests/acceptance` by +7.5M (328.5M → 335.9M), deterministic. Reverting restored 328.5M.

A genuine win requires the READ side to consume the typed `Imm` directly (match `Operand::Imm(n)` via `operand()`) instead of `rendered().parse::<usize>()` — a read-side change, not a producer swap. See the note on CodeInstruction operand typing / regalloc perf (allocation-bound acceptance compile).

## CodeInstruction operand typing & regalloc perf (registers store as Raw)

`CodeInstruction.fields` is `Vec<(&'static str, Operand)>` (`Operand` = `VReg{class,id}` | `Imm(i64)` | `Raw(Box<str>)`, `src/target/shared/code/operand.rs`). `MirInstruction.fields` stays `String`-valued — the `select`/`lower_to_mir` boundary converts via `mir_fields_from_code` / `code_fields_from_mir`. `get()` renders (`Option<String>`); `operand()` returns `&Operand`; `field()` takes `impl Into<Operand>` (`&str`/`&String`/`String`/`&&str` → `Raw`). `Operand: Display + PartialEq<str>` so string-shaped readers keep working; hot reads use `rendered()` (Cow, borrows a `Raw`).

Register operands are stored as `Raw` strings, NOT typed `VReg`/`Phys`. `allocate_register`/`temporary_vreg` return `String` consumed by ~1794 call sites, so typing vregs at their source is out of scope for a single plan — `VReg`/`Phys` are the test-only typed surface. Physical names also stay `Raw` (a `Phys{class,index}` can't render faithfully: `x0`/`rax`/`zero` all = int index 0; `d3`/`v3` alias fp index 3). Immediates ARE typed `Imm` at the `finalize_frame` offset rewrite.

The compile is ALLOCATION-BOUND — measured on the real workload (`mfb test tests/acceptance`, 4071 lines, macOS aarch64). Release acceptance 58s, debug 284s, of which codegen+link is 97–99% (front-end ~2s, runtime ~1s — both noise). A counting `GlobalAlloc` shows 808,808,429 heap allocations / 37 GiB churn for that one compile; `sample` puts 74% of release self-time in malloc/free/memmove. It splits by substage (env-gated timers in `macos_aarch64/mod.rs` write_executable): code_emit `code::lower_module` 67% / 566M allocs, encode 23% / 215M allocs, everything else <3%. The `plan+regalloc` LABEL is only 58ms — misleading: `regalloc::allocate` actually runs INSIDE code_emit (called from `builder_registers.rs` + `codegen_utils.rs`), and its hot subtree is `linear_scan::run`→`Vec::clone`→`Box<str>::clone`→malloc. Root cause = the per-operand `Box<str>` (registers stored as `Raw`): production (`allocate_*`/`temporary_vreg`→String, 1825 sites), regalloc cloning it per instruction, and encode's `REG_ARRAY.position` string scans.

In DEBUG the allocator is only 19% (81% is mfb's own unoptimized code) but the top self-costs are the SAME string work — `eq`, `position`, render — so the fix (type registers as inline `{class,u32}`, not `Box<str>`) helps both. An earlier "distributed cost / micro-opts suffice" reading came from profiling ONE 860k-instr regex fn (unrepresentative: acceptance is 841 fns, one 4.99M-instr fn = 51% of 9.79M total instrs); its `%`-fast-reject + `U32Hasher` + sweep were real and byte-identical but netted ~4% because they shave the 26% compute, never the 74% allocation. Type the registers — do NOT re-try analysis micro-opts. The `U32Hasher` swap is byte-identical only because every order-dependent iteration is sorted first.

## A String register spelling erases typed ABI tokens

A convention ABI token (`Operand::Abi`, e.g. `%argMFB0`) is realized directly by each backend's typed `Operand::Abi` handler (`realize_abi_positional` on AArch64/RISC-V, `realize_abi_operand` on x86). But ANY codegen path that stores or renders a register operand as a `String` silently downgrades it to `Operand::Raw`, and a Raw convention token needs a string realizer to become a physical register.

The three erasure sites (all fixed so the string realizers `realize_convention_token`/`map_convention_token` could be deleted):
1. `ValueResult`/`CodeParam`/`PendingTemp.location` were `String` — a parameter's location (`argument_register(i)` = `%argMFB0`) emitted via `move_register(dst, &loc)` as a Raw token. Typed to `Operand`.
2. riscv64 `expand_fused` (fused compare/overflow) read setter operands via `.render()`. Preserve the `Operand`.
3. riscv64 `addr_of` (auipc/addi) rendered its `dst`. Preserve the `Operand` (AArch64's addr_of already used `&dst` typed — that's why the leak was riscv-only).

Why it's dangerous: the leak is INVISIBLE to the `.ncode` golden — a `Raw` `%argMFB0` and a typed `Operand::Abi{...}` render to the identical string, so the neutral dump is byte-identical either way. It only surfaces at the machine-code encoder (`unknown register '%argMFB0'`) or as a `.ncodesum` (machine-byte) diff. So after any ABI-token/codegen change: run the FULL artifact-gate (it caught the riscv64 addr_of leak as a `.ncodesum` diff that the `.ncode` check missed), not just cargo test — and remember cargo test's `cli::build` only exercises the HOST arch, so a riscv-only leak passes cargo test.

## FOR-loop desugar creates synthetic locals

The IR lowers a source `FOR i = 0 TO <bound> [BY <step>]` into synthetic locals, NOT into a loop that names `i`/`bound`/`step` directly. Confirmed by `-nir`:

```
{ "op": "bind", "name": "$for_end1",  "value": <bound expr, e.g. n - 2> }
{ "op": "bind", "name": "$for_step2", "value": <step expr, e.g. const 1> }
{ "op": "for", "name": "$for_iter0",
  "start": const 0,
  "end":   { "kind": "local", "name": "$for_end1" },
  "step":  { "kind": "local", "name": "$for_step2" },
  "body":  [ { "op": "bind", "name": "i", "value": { "kind": "local", "name": "$for_iter0" } },  // <-- the user var is an ALIAS
             ... uses `i` ... ] }
```

So `lower_numeric_for` receives `name = $for_iter0` (not `i`), and `end`/`step` as `Local($for_endN)`/`Local($for_stepN)` (not the `n-k`/`1` exprs). The user's `i` is a separate `LET i = $for_iter0` alias, and the body indexes with `i`.

Consequences for any FOR-loop dataflow pass (a bounds-check elision hit all three):
1. Resolve the bound/step through the synthetic binds (track `$for_end*`/`$for_step*` bind values) before pattern-matching `end == len(L) - k` / `step == 1`.
2. The induction var is `$for_iterN`, not the source name. A fact recorded on `$for_iter0` will NOT match the body's `get(L, i)` — the body uses the alias `i`.
3. Propagate the fact through the `LET i = $for_iterN` alias (in the Bind handler, when `Bind{i, Local(src)}` and `src` carries the fact, copy it to `i`). Clear it on any rebind/assign of `i` so a later reuse of the name can't inherit a stale fact.

Register/label churn note when adding such a pass: allocating a `temporary_vreg` renumbers `next_vreg` and churns EVERY golden (see the vreg-alloc-order note); adding a `self.label()` only renames labels and does NOT churn `.ncode` bytes (labels resolve to offsets) — so reuse existing free scratch registers, but new labels are fine. Keeping a register allocated-but-unused (e.g. `count` when the bounds check is elided) preserves the numbering, so the diff is only the removed compare/branch instructions.

## Monomorph fan-out: DFS front-loads depth

Monomorphization bounded instantiation DEPTH (`MAX_TEMPLATE_INSTANTIATION_DEPTH=256`) but not BREADTH. A generic recursing through ≥2 type-widening self-calls (`recurse<T>` → `recurse<List OF T>` + `recurse<Set OF T>`) fans into an exponential tree of distinct `name<args>` keys the depth cap never collapses; the per-leaf cap returned `None` without halting enumeration → never terminates.

Non-obvious mechanics:

- DFS front-loads depth. Lowering is depth-first, so the first path descends straight to depth 256 (≈256 instantiations) and trips `TYPE_INSTANTIATION_TOO_DEEP` before a several-thousand total budget is ever reached. So for any self-recursive fan-out the DEPTH cap fires first; a pure total budget alone would still emit a handful of depth diagnostics per sibling leaf. The real fix was halt-on-first-limit: latch `instantiation_limit_reached` the moment either limit trips so the rest of the (exponential) tree is pruned → single diagnostic, prompt exit.
- The total budget (`MAX_TOTAL_INSTANTIATIONS=4096`) only fires for bounded-depth WIDE programs (a finite fan-out chain `f1→…→fN` deep <256 but wide), never for the classic deep self-recursion.
- Two widenings must be non-collapsing to fan out. `List` + `List OF List` COLLAPSE (paths converge on nesting depth → memoized to ~hundreds). `List` + `Set`, or two distinct user wrappers `L`/`R`, do NOT commute → true 2^k tree.
- Reaching a several-thousand budget is inherently ~seconds (each deepening instantiation does O(type-size) string work), so a budget-tripping end-to-end fixture is too slow for CI — test the counter with a white-box unit test (`charge_instantiation` in a loop) instead; keep the committed fixture on the fast depth-halt path.

Rule: `2-203-0135 TYPE_INSTANTIATION_BUDGET_EXCEEDED`. Threat model is DoS on an untrusted `.mfb`/build file the victim compiles.

## A rename fix can leak a mangled name into a diagnostic

Sweeping a previously-skipped reference through the file-PRIVATE `rename` map (`src/ast/scope_privates.rs`) is only half the fix when a downstream diagnostic interpolates that name. Before the fix, `g.state = v` on a PRIVATE `g` left `StateAssign.resource` bare, so the resolver reported a bogus `SYMBOL_UNKNOWN_IDENTIFIER` on `g`. After sweeping `resource`, the name reaches the checker mangled (`#<hash>$g`), and `syntaxcheck/checking.rs`'s `TYPE_UNKNOWN_VALUE` arm interpolated it raw — so the "fix" would have leaked the untypeable internal spelling into the user-facing message (arguably worse than the original wrong diagnostic).

Why: `mangle_private` produces `#<hash>$name`; user-facing diagnostics must never show the sigil form. `crate::internal_name::display_name(name)` demangles `#<hash>$name` → `name` (and `#pkg_helper` → `__pkg_helper`) and is a no-op on non-internal names.

How to apply: when a diagnostic names a symbol that could be a mangled PRIVATE/internal name, wrap it in `internal_name::display_name(...)`. At the time of the fix, `display_name` was only used in `monomorph/lower.rs` — most checker/resolver messages interpolate names raw and are latent leaks the moment a mangled name can reach them. Verify with a full-pipeline `tests/syntax/*` fixture (golden `build.log`), not `check_src`, since the checker-only testutil does NOT run `scope_privates`.

## Recursion over a LOCAL union lowers; over an IMPORTED union it does not

The "walk trees with an explicit work-stack, never a recursive function over the union" rule is NARROWER than it sounds: it applies to a package that imports a union and recurses over it. A recursive `FUNC` over a union defined in the same package lowers to native and runs correctly.

Verified (macos-aarch64, target/release/mfb): a package `rec` defining `UNION Tree { Leaf, Branch }` with `EXPORT FUNC sumTree(t AS Tree) AS Integer` that recurses (`FOR EACH k IN b.kids: total = total + sumTree(k)`) builds as a package, and an app importing `rec` and calling `rec::sumTree` builds AND runs (prints the correct sum). The recursion is internal to `rec`; the consumer never recurses.

Consequence for the browser example: a DOM-transform pass (e.g. style resolution filling `ElementNode.style`) that lives inside `dom` may recurse over `Node` directly — no manual stack needed. Only `display`/`fetch`/`app` (which import `dom::Node`) must stay iterative.

## TRAP inside a MATCH CASE mis-types its temp as Unknown

A trap-bound producer written directly inside a `MATCH CASE` body — `MUT x AS T = <call> TRAP(e) … RECOVER … END TRAP` inside `CASE Variant(v)` — passes `-ast -ir` but fails native codegen with:

    error: native plan has no storage class for type 'Unknown'

The `TRAP` desugars to a temp local (`bind x = local $trap_valN`); inside a MATCH-CASE scope that temp's type isn't registered, so the code layer reads it as `Unknown`. The SAME `<call> TRAP … RECOVER []` works fine at a function's top level (e.g. `net`/`tls` exchange loops use it).

Workaround: factor the trap-bound read out of the MATCH into a top-level helper (one per variant) that the CASE only calls — no `TRAP` directly in the CASE. This is cleaner anyway and sidesteps the bug. The underlying codegen bug (register the MATCH-CASE-scoped trap temp's type) is unfixed; repro = the pattern above.

## Regex parser depth cap must be lower than the matcher's

The mfbasic regex/json engines have TWO independent stack-overflow guards, and they need DIFFERENT limits — do not reuse one for the other.

- The matcher (`__regex_matchNode`) uses `__REGEX_DEPTH_LIMIT = 600`: it recurses ~1 native frame per depth unit, native stack exhausts around 800–1000 frames.
- The parser (`__regex_parseAlt → parseConcat → parseParen → parseAlt`) recurses 3 native frames per group-nesting level. Measured on the produced executable (macOS-aarch64): `(`×N compiles cleanly for N≤300 but SIGSEGVs (exit 139) for N≥400 — the crash threshold is ~350 nesting levels ≈ ~1050 frames.

So a depth-600 check in the parser would NEVER fire before the ~350-level crash. Reusing `__REGEX_DEPTH_LIMIT` was wrong. The fix added a parser-specific `__REGEX_PARSE_DEPTH_LIMIT = 200` (same ~600-frame budget ÷ 3 frames/level). Verified at the boundary: N=200 balanced groups compile, N=201 fails cleanly, N=800/2000 fail cleanly — no crash at any depth.

General rule: when capping recursion depth to prevent an uncatchable native SIGSEGV, calibrate the limit to native FRAMES consumed, not to the logical construct — count how many native frames one unit of the recursion cycle actually costs, and set the limit against the measured crash threshold of the produced binary.

Regeneration note: a change to a builtin `*_package.mfb` also shifts `tests/byte-identity/<pkg>`'s per-target `.ncodesum` (sha256 of the `.ncode` dump). Those are regenerated ONLY by the artifact-gate path — `mfb build -ncode -target <t>` then `shasum -a 256 | cut -d' ' -f1` into the golden, once per target (macos-aarch64 host needs no `-target`). `scripts/test-accept.sh`/`sync-goldens.sh` NEVER touch `.ncodesum` (it is not in `ARTIFACT_NATIVE_KINDS`); `artifact-gate.sh` only DIFFs, it has no accept mode. Verify the `.ir` delta by normalizing BOTH `"line": N` and the `ErrorLoc` embedded source-line integer constants before diffing — a 13-line insertion otherwise looks like ~18k changed lines (every embedded source line renumbers).

## Arena free-list goes quadratic on mixed-size transient churn

The runtime arena's free list degrades quadratically under mixed-size transient churn, and the degradation is process-global and cumulative — a fresh loop starts fast, each subsequent repeat/call gets dramatically slower until it hangs.

Triggers (short-lived temporaries of several distinct sizes, alloc+free each iteration):
- `strings::graphemes` / `graphemesCount` / `graphemeAt` / `toBytes` / `normalizeNfc` in one loop. (graphemesCount/graphemeAt are also inherently ~230µs/call.)
- `collections::sort` and `collections::window` over String lists (many String copies). `chunks`/`distinct`/`zip`/`take`/`drop`/`mid` over String lists stay fine at the same counts.

Not triggered by: uniform single-size allocation loops. `graphemes` alone = 23ms/20000, `toBytes` alone = 11ms/20000, `normalizeNfc` alone = 147ms/20000 — all linear. A 1M-append Integer/String list churn does NOT degrade a later grapheme loop. It's the mix of sizes freed repeatedly that fragments the free list.

Measured blow-ups: benchmark `liststr reshape` at base=1000/k=50 = 458 s; `string unicode` at 20000 iters = ~19 min (98% CPU, real compute). Same loop at base=40/k=3 or unicode inner=10 is flat (2ms) across 10 runs.

Repro shape: `FOR r ... FOR i=0..N { g=graphemes(u); ...=toBytes(u); ...=normalizeNfc(u) }` — times per run climb 21→146→409→833→…→16818ms as cumulative allocations grow.

Likely a residual path not covered by the earlier mixed-churn fix (arena-allocator-quadratic-mixed-churn) / large-block bins. The benchmark works around it by keeping those two rows at coverage-only counts. A real fix belongs in the arena free-list allocator (src/), not the benchmark.
