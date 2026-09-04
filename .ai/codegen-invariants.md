# Codegen & IR invariants

Load-bearing facts about the MFB compiler's native code generation and IR lowering (aarch64 / riscv64 / x86-64 / macOS / Windows). Each section captures one invariant: the mechanism, the failure it causes, and the fix.

## Types below the AST are `ParameterType`, with a nominal-name string boundary (plan-104)

Every type-spelling STORE below the front end is `crate::types::ParameterType`, not `String`: the IR fields (incl. `IrOp::{Bind,For,ForEach}.type_`), every NIR field (`src/target/shared/nir/mod.rs` — the `.nir` JSON dump renders `name()` at the emit point only), and codegen's own stores — `ValueResult.type_`, `LocalValue.type_`, `GlobalValue.type_`, `FieldTypes`, `TypeModel.record_fields`/`union_variant_fields` values, `package_return_types`, ALL FOUR NIR type oracles (`static_nir_value_type`, `static_type_name`, `static_type_name_with_types`, and the two `_for_fold` twins — every one `Option<ParameterType>` since plan-106-E; promotion goes through `type_utils::promoted_binary_type`, a one-line delegation to the single algebra in `src/numeric.rs`). Structural questions (element/key/value/return types, is-collection, is-FUNC) are variant matches / the `typed_*` twins in `engine/types/type_utils.rs` — never `strip_prefix("List OF ")` on a rendered name. The registry resolves typed calls via `resolve_call_typed` / `resolve_call_return_type_typed` (no render/parse for generic members).

The deliberate STRING boundaries that remain — render `name()` at these sinks, and do not "fix" them by parsing back:
- The **nominal-name domain**: `TypeModel` maps are keyed by type NAMES, the layout/value-semantics classification web (`type_is_flat`, `record_field_is_inlined/pointer`, `CollectionTypeLayout::from_type`, the payload emitters) recurses over nominal names and `X STATE Y` composites — its `&str` params are name-domain sinks like symbol tables.
- Mangled `$`-suffix fragments (`#collections_chunks$T`), symbol names, error/diagnostic text, `ValueResult.text`.
- Wire strings: `.mfp` binary encode/decode (decode parses once), `IrLinkFunction` LINK types, the registry's string wrappers for the still-string front/middle end, and the bespoke `general`/`vector`/`strings` resolvers.

`parse ∘ name = id` is load-bearing across every one of these seams; a construction like `ParameterType::parse(&format!("List OF {element}"))` is behavior-safe but directionally wrong when a typed head exists — prefer `ParameterType::list_of(element.clone())`.

## `ParameterType::parse` is the ONLY type-grammar implementation (plan-105-B)

`src/types.rs` owns the type grammar. The only other legitimate parser is the **source-language** one in `src/ast/` (tokenizer-side, produces the AST). Everything else — resolver, monomorph, the shape pass, IR lowering, `ir::verify`, codegen — MATCHES on `ParameterType` variants; it does not re-implement the grammar. The private `strip_prefix("List OF ")`-style cascades that used to live in `monomorph::helpers` (`user_template_parts`, `func_type_parts`, `split_top_level_to`, `split_top_level_commas`), `resolver::resolution` and the former source checker are deleted (the checker itself went with plan-107-D).

Two traps this cost real time to learn:

- **The canonical parser was not automatically the correct one.** Before plan-105-B, `parse`'s `Map OF` / `MapEntry OF` arms split on a leftmost `rest.split_once(" TO ")`, while the three private copies each carried a **depth-aware** scan — the bug-108.2 / bug-41 fix for a key that itself carries a top-level ` TO ` (`Map OF Map OF String TO Integer TO Boolean`). Consolidating onto `parse` would have silently regressed both bugs. The depth-aware scan is now `crate::types::split_top_level_to` and `parse` uses it. **Before routing callers onto a "canonical" helper, diff it against the copies you are deleting** — the duplicate may be the one carrying the bug fix.
- **A user generic is `ParameterType::UserOf(Symbol, Vec<ParameterType>)`, not a `Named` blob.** `Pair OF Integer, String` decomposes structurally, so `unify`/`substitute`/`contains_var`/`unify_type`/`substitute_type_params` recurse into the arguments like any container. `parse`'s `UserOf` arm is ordered AFTER every built-in ` OF ` constructor, and `split_user_generic` additionally rejects built-in heads by name, so a malformed `Map OF K` (no ` TO `) is never re-read as a user template named `Map`. `Thread`/`ThreadWorker` keep their own `ThreadHandle` variant — they carry the RES/STATE planes.

Adding a variant means wiring, in lockstep: `parse` arm, `name()` arm, `with_vars`, registry `unify` + container fail-set + `substitute` + `contains_var`, and monomorph's `unify_type` / `substitute_type_params`. Nothing errors if you miss one — every consumer has a `_` catch-all, so an unwired variant silently falls into wildcard behavior. Grep an existing variant (`MapEntryOf`) to enumerate the sites.

**Measured (plan-111-A Phase 3, adding `Stateful`): the tree has 81 `match`es on a `ParameterType` with a top-level `_` arm, and adding a variant compiles CLEAN — zero exhaustiveness errors.** So the compiler tells you nothing; the audit is yours. What makes it tractable is asking the right question: a new variant only changes behavior where the value **used to take a different arm**. `File STATE Cursor` reached all 81 as `Named(...)`, so only the sites with their own `Named(..)` arm can move — **7 of 81**. (Script: walk each `match` block by brace depth, keep the ones whose top-level arm *patterns* name `ParameterType::`, then split on whether any pattern names the variant the value used to be.) Do not forget the non-`match` forms — `if let`, `matches!`, `while let` — which a `match`-block scan misses entirely; there were 5, found with `rg 'if let (Some\()?(ParameterType|Self)::Named|matches!\([^,]*, *(ParameterType|Self)::Named'`.

**That census still missed the two sites that actually broke (plan-111-G6).**
Both were `Named(_)` guards meaning "is this a nominal?", and both silently
changed answer when `File STATE Cursor` stopped being a `Named`:

* `let ParameterType::Named(_) = matched_type else { return false };` — a
  **`let`-else**, which is neither a `match` block nor any of the three
  non-`match` forms the `rg` above looks for. Add `let ... else` to the sweep.
  This one stopped `ir::shape` binding `CASE Variant(v)` over a stateful union,
  so `v.state` typed `Unknown` and the error surfaced two layers away as
  `toString(Unknown)`.
* A sibling defect that no variant sweep can find: a conversion that changes a
  table **KEY** from the full type to `resource_base_type(...)`. `File STATE
  Cursor` is absent from the record table (so the access stayed unchecked); the
  bare `File` base is present, because a resource declares inline fields — so
  `.state` was rejected on every stateful resource. Retyping a lookup, always
  ask what the OLD key was, not what the new one reads more nicely as.

Neither failed to compile and neither reddened a unit test. The signal was one
acceptance fixture that stopped building, found only by the full cross-target
gate. **Budget an `artifact-gate all` run for any variant addition.**

## `tests/no_type_strings.rs` is a hard floor with two named remainders (plan-111)

It scans `src/` (minus `src/ast`, `src/lexer.rs`, `src/docs` — the string domain) for **eight** ways a type SPELLING reaches a decision: `ParameterType::parse` outside the boundary files, `ParameterType::declared` (class 1b — where a declared NAME crosses into the type domain), a type-named `&str` parameter, a `match` arm or `==` on a spelling, a hand-rolled grammar op, a `format!`-built spelling, and a `String`-keyed type map.

Since plan-111-G it is a **floor, not a ratchet**: six of the eight classes are 0 tree-wide, and `BUDGETS` is down to two classes whose remainder is enumerated site-by-site in the table's own comment. Two exemptions carry the rest, each pinned by its own test:

- `is_grammar_file` — `src/types.rs` DEFINES `parse`/`name`, so it is totally exempt. Exactly one file (`the_grammar_file_is_exactly_one`).
- `is_boundary_file` — the six boundaries (in five groups: the parser, the AST→HIR seam, the `.mfp` codec both directions, the IR binary codec, the manifest) are exempt from the four NAME-HANDLING classes and from **none** of the three DECISION classes. A boundary may parse a spelling and may render one; it may not decide anything by comparing one. The exact membership is asserted (`boundary_list_is_closed`), so a seventh entry is a deliberate edit, not a quiet one.

Two things to know before you touch it:

- **The table is asserted tight in BOTH directions.** A count above its budget fails with every offending `file:line`; a budget *above* the live count also fails with "lower this budget to N". So clearing sites without lowering the row is a red test, and so is lowering a row too far. Lower it **in the same commit as the work**, and take the number from the failure message — it prints the whole live table paste-ready.
- **It does not use `architecture_guards.rs`'s `code_above_tests`.** That helper truncates a file at its first `#[cfg(test)]`, which is safe for `src/codegen` + `src/target` but wrong across `src/`, where `#[cfg(test)]` also sits on mid-file items (`ir/shape.rs`'s `bound_types`, `resolver/mod.rs`'s `resolve_hir_project`) — it scanned 4202-line `shape.rs` down to 158 lines. `test_free_lines` strips each `#[cfg(test)]` item by brace depth instead. `architecture_guards.rs` still carries the naive version; harmless only because of its narrower roots.

The `string_keyed_type_maps` class is a **curated** `(file, identifier)` list, not a regex: the broad needle matches 1209 lines in `src/`, nearly all keyed by a symbol (function name, binding name, package alias), which is legitimately a string. Its doc comment names the four nearest non-type-keyed lookalikes so they do not get "fixed".

### User-generic limitations that are GRAMMAR, not bugs

`Holder OF Pairing OF Integer, String` does not compile (`cannot infer template arguments`), and it cannot be fixed in the parser: the spelling is textually indistinguishable from a two-argument `Holder OF (Pairing OF Integer), String`, and the language has no bracketing. `parse` splits on top-level commas and yields 2 arguments for a 1-parameter template; it holds no arity table and is deliberately dependency-light. Same root cause: a user-generic-typed **parameter must come LAST** in a parameter list — `FUNC f OF T(b AS Box OF T, v AS T)` does not parse, because `OF`'s argument list is read greedily across commas. Write `FUNC f OF T(v AS T, b AS Box OF T)`.

Three levels of user generic (`Holder OF Holder OF Holder OF String`) also still fails (`SYMBOL_UNKNOWN_TYPE: Type 'Holder'`): a doubly-nested constructor's expected type is the enclosing template's already-mangled field type, which no longer names a template. Two levels work. Coverage for all of this lives in `tests/rt-behavior/generics/` — which did not exist before plan-105-B; the corpus had **zero** `TYPE X OF T` fixtures, so this grammar was entirely ungoldened.

## Operators are a closed enum with exactly two mint sites (plan-112)

`BinaryOp` (17 variants) and `UnaryOp` (3) in **`src/operators.rs`** are the compiler's only operator representation from the token the parser consumes to the byte codegen emits. Unlike `ParameterType` the vocabulary is **closed** — no operator overloading, no user-declared operator, no path from an identifier to one — so there is no interner, no `Symbol`, and no `UserOf` escape hatch: a `u8`-sized `Copy` enum is the whole representation.

**Two enums, not one, because arity is real.** A single `Operator` would re-admit `Binary { op: Not }`. `UnaryOp` has deliberately **no `Plus`**: no parser path mints a unary `+`, and two stages carried dead arms handling one until the missing variant deleted them.

**The mint sites are `BinaryOp::from_token` (via `src/ast/expr.rs`'s precedence ladder) and the two direct constructions in `src/ast/link_items.rs`** (`ERROR_ON`'s De Morgan `NOT`, and the `SIZEOF` const pin). Nothing else constructs one from text except `parse` at the decode boundary.

- **`name()` is a wire format, not a display.** It is rendered verbatim into three committed sinks — `.ast` JSON (`ast/serialize.rs`), `.ir` JSON (`ir/json.rs`), and the length-prefixed operator string in the `.mfp` binary (`ir/binary.rs`) — and pinned by 1586 goldens. Changing a returned string is a format break. There is deliberately **no `Display` impl**: every render goes through `name()` so the sinks are greppable.
- **`parse()` is for decode boundaries only.** `ir/binary.rs` rejects an out-of-vocabulary operator at decode rather than mis-lowering it (bug-403's guarantee, extended from `IrLinkExpr` to `IrValue`); `ir/link.rs`'s `link_compare_op_valid` is the same check for the LINK wire. No compiler stage reaches for `parse` to make a decision.
- **`IrLinkExpr::Compare { op: String }` is a fourth carrier that stays a `String` on purpose** — bug-403's regression test constructs a garbage one and must keep compiling. Its two decision sites route through `BinaryOp::parse`/`is_comparison`, so the file decides nothing by spelling.
- **Arity conflation is the trap this exposed.** Several predicates consulted **one** `&str` list in which `"-"` meant subtraction *and* negation at once, and which one a lookup meant depended on the caller (`fallible::operator_can_raise`, `shape::note_short_circuited_operator`). If you add a shared operator predicate, split it by arity.
- **`src/optimizer/opt2/**` and `src/arch/**` are a DIFFERENT vocabulary.** Their `op` is a `CodeOp` machine mnemonic (`"adrp"`, `"fadd_d"`), already an enum, and out of scope. Only `src/optimizer/opt1/**` operates on language operators.
- **`SIZEOF` had zero golden coverage** until `tests/byte-identity/link-const-pins` gained a `CONST nbyte = SIZEOF CPinnedInfo` pin. It is LINK-only and folds to its integer during LINK lowering, so it reaches the `.ast` dump and nothing after it — the `.ir` never sees it.

`tests/no_operator_strings.rs` holds this as a **hard zero with no budget table** (the vocabulary is closed, so there is no legitimate remainder): no spelling in a `match` arm or `==`, no `Binary`/`Unary`/`Compare` node carrying a `String` operator, no fn taking an operator as `&str`. Two allowances, each pinned by its own test: `src/operators.rs` (defines `name`/`parse`, total exemption, exactly one file) and a closed nine-entry list for the classes-3-and-4 lookalikes — the `ir/link.rs` wire boundary plus the eight files where an identifier called `op` is a mnemonic or a member name. Spelling *decisions* are exempt nowhere, including in those nine. The list is asserted tight in both directions: an entry that no longer has a site fails as stale.

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
4. **A fact established OUTSIDE a loop is re-used on every back edge of that loop (bug-495).** Lowering is one linear pass, so "clear the fact when `L` is reassigned" runs only when the reassignment is *lowered* — which, for `FOR r … / FOR i = 0 TO n-1 … NEXT / xs = [99] / NEXT`, is AFTER the inner `FOR` has already been emitted unchecked. At run time the outer back edge re-enters the inner loop with `n` stale and `xs` shorter: a heap OOB read from a 15-line program. The body-only no-reassign proof cannot see this; `lower_loop_body` pushes every loop body's `collect_reassigned_locals` set on `enclosing_loop_reassigned`, and a `LET n = len(L)` fact carries the stack depth it was recorded at, so the recognizer declines whenever a loop entered *after* the fact reassigns `L`/`n` anywhere in its body. A loop that *contains* the `LET` re-runs it each iteration and is exempt (index `< depth`), so the plan-86-G `listchurn` shape and a nested loop that leaves `L`/`n` alone still elide. Only the direct `FOR i = 0 TO len(L)-k` bound is depth-free (re-evaluated at every entry). Negative fixture: `tests/rt-error/collections/bounds_elim_backedge_rt`; controls: `tests/rt_bounds_elim_backedge.rs`.

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

Sweeping a previously-skipped reference through the file-PRIVATE `rename` map (`src/ast/scope_privates.rs`) is only half the fix when a downstream diagnostic interpolates that name. Before the fix, `g.state = v` on a PRIVATE `g` left `StateAssign.resource` bare, so the resolver reported a bogus `SYMBOL_UNKNOWN_IDENTIFIER` on `g`. After sweeping `resource`, the name reaches the checker mangled (`#<hash>$g`), and the checker's `TYPE_UNKNOWN_VALUE` arm (today `ir::shape`'s `check_initializer_known`) interpolated it raw — so the "fix" would have leaked the untypeable internal spelling into the user-facing message (arguably worse than the original wrong diagnostic).

Why: `mangle_private` produces `#<hash>$name`; user-facing diagnostics must never show the sigil form. `crate::internal_name::display_name(name)` demangles `#<hash>$name` → `name` (and `#pkg_helper` → `__pkg_helper`) and is a no-op on non-internal names.

How to apply: when a diagnostic names a symbol that could be a mangled PRIVATE/internal name, wrap it in `internal_name::display_name(...)`. At the time of the fix, `display_name` was only used in `monomorph/lower.rs` — most checker/resolver messages interpolate names raw and are latent leaks the moment a mangled name can reach them. Verify with a full-pipeline `tests/syntax/*` fixture (golden `build.log`), not `check_src`, since the checker-only testutil does NOT run `scope_privates`.

## Recursion over a LOCAL union lowers; over an IMPORTED union it does not

The "walk trees with an explicit work-stack, never a recursive function over the union" rule is NARROWER than it sounds: it applies to a package that imports a union and recurses over it. A recursive `FUNC` over a union defined in the same package lowers to native and runs correctly.

Verified (macos-aarch64, target/release/mfb): a package `rec` defining `UNION Tree { Leaf, Branch }` with `EXPORT FUNC sumTree(t AS Tree) AS Integer` that recurses (`FOR EACH k IN b.kids: total = total + sumTree(k)`) builds as a package, and an app importing `rec` and calling `rec::sumTree` builds AND runs (prints the correct sum). The recursion is internal to `rec`; the consumer never recurses.

Consequence for the browser example: a DOM-transform pass (e.g. style resolution filling `ElementNode.style`) that lives inside `dom` may recurse over `Node` directly — no manual stack needed. Only `display`/`fetch`/`app` (which import `dom::Node`) must stay iterative.

## The inline-TRAP region is built in `ir::lower`, NOT in codegen

An inline `TRAP` has no backend op: `ir::lower::lower_inline_trap` desugars it into
`Bind $trap_resN` / `If ResultIsOk` checks. Anything that must be *covered* by the
handler therefore has to be lifted there — codegen has no notion of "the ops of this
trap region". Two kinds of node are lifted, and they use different wrappers:

* a fallible **call** becomes `IrValue::CallResult` (bug-457);
* a raising **operator** becomes `IrValue::Checked { type_, value }` (bug-471) —
  "evaluate `value` with its domain-error exits captured, yielding `Result OF type_`".

`Checked` works because `emit_error_register_return` already consults
`raw_result_capture` (the per-value redirect `lower_inline_conversion_raw` /
`lower_inline_builtin_raw` set around ONE builtin); `lower_checked_value` just sets it
around an arbitrary value instead. Three traps around it:

* **A `Checked` operand must be call-free.** A callee's error return does not pass
  through `emit_error_register_return` in *this* frame, so it would auto-propagate
  straight past the capture. The desugar lifts every call out first, and
  `ir::verify::check_checked_has_no_call` rejects the shape on the decoded-package path.
* **`Checked` is the observation boundary for a `Float`.** plan-17 moved `+`/`-`/`*`/`/`'s
  finiteness check from the operator to wherever the value is first consumed. Once
  lifted, the operator feeds a `ResultValue` — not an arithmetic node — so nothing
  downstream observes it: `lower_checked_value` must call `observe_float` INSIDE the
  capture or an overflow to infinity is delivered as a finite-looking `Ok`.
* **A negative literal is `Unary(-, Const)`, not a computed negation.** It is by far the
  commonest operator inside a trapped expression (`f(-1) TRAP`), and lifting it costs a
  whole `Result` materialization to check a negation that provably succeeds — it was the
  ONLY thing that showed up in all 8 `.ir` golden diffs when bug-471 first landed without
  the carve-out. `fallible::is_total_literal_negation` exempts it, excluding `Byte`
  (whose negation raises `ErrUnderflow` for any non-zero operand). The i64::MIN spelling
  is safe because lowering folds `-9223372036854775808` to a single `Const`.

The scan and the rewrite (`scan_trap_call` / `rewrite_trap_call`) must agree, node for
node, on which nodes are indexed — a position means `fallible[position]` to one and
"lift or leave" to the other. Both ask the single `trap_hoist_kind` predicate; a
`debug_assert_eq!` pins the agreement, so CI (which runs DEBUG) is where a desync
surfaces.

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


## A type argument is a `ParameterType`, and the ratchet gate has known edges

Since plan-111 an emitter takes `&ParameterType`, never a rendered spelling, and
`tests/no_type_strings.rs` enforces it with per-`(class, directory)` budgets that
only ever shrink. Two things about that gate are worth knowing before trusting a
zero:

**1. It had three blind spots, fixed in plan-111-D; assume there are more.**
The scanners are text heuristics, and each miss hid live sites for three whole
letters:

* `spelling_match_arms` required the arm to *begin* with a quoted spelling, so a
  **tuple arm** — `("sin", "Float") => FloatKernel::Sin` — was invisible. It now
  scans the whole arm pattern, stopping at ` if ` (a guard decides by
  *comparing*, which is class 4's needle, not class 3's).
* `spelling_compares` matched `== "Integer"` but not the **wrapped** form
  `== Some("Integer")`.
* `TYPE_PARAM_NAMES` was hand-seeded and missed ten `*type*: &str` parameter
  names (`record_type`, `result_type`, `stride_type`, `block_type`, …).

Those three fixes surfaced **59 sites** and a new `optimizer` bucket. If you add
a needle class or a spelling, re-run the census rather than assuming the budget
table is the population.

**Measure with the gate, not with `rg`:**

```
cargo test --test no_type_strings census_by_file -- --ignored --nocapture
MFB_CENSUS_DETAIL=<path-substring>   # adds every offending line
```

`rg` counts inline `#[cfg(test)]` modules and over-reports (one plan's census
read 84 where the production population was 21). The gate's `test_free_lines`
stripper does not.

**2. Convert a producer and its consumers together.** A type that is *written*
as a spelling and *read* as one is a single site, however far apart the two ends
are. `target/shared/abi.rs`'s `move_immediate(type_: &str)` writes the NIR
`mov_imm` operand-class attribute; `optimizer/opt2/{lvn,gvn,constant_folding}.rs`
read it back with `instruction.get("type") == Some("Integer")`. Converting
either end alone leaves the other matching a spelling that is no longer written
the same way.

**3. At a boundary with a not-yet-converted cluster, render ONCE and say so.**
The codegen `&str`-type plane is one connected graph, and plan-111's letters cut
across it. When a converted function calls a still-untyped helper
(`list_entry_stride`, `emit_inlined_block_size_from_ptr_slot`,
`inline_collection_payload_size` — all keyed by a spelling), put a single
`type_.name()` at that call and name the letter that deletes it. What this rule
forbids is the opposite move: typing a signature and pushing the render *out* to
its callers, which multiplies renders while the gate count goes down.

## Two flatness predicates, not one (plan-114-B)

`type_is_flat` is gone. It answered two questions with one predicate:

- `type_is_memcpy_copyable` — "does a `memcpy` of this block COPY it correctly,
  within one thread?" Consumers: `is_pointer_collection_payload_type`,
  `list_element_padding_alignment`, `record_field_is_inlined`,
  `is_freeable_flat_value`, and `copy_value_to_current_arena` (which copies into
  the CURRENT arena — see below).
- `type_is_arena_transferable` — "may this block be RELOCATED into another
  thread's arena?" Strictly stronger: a resource handle anywhere inside would
  arrive pointing into the *sender's* arena. Consumers:
  `collection_payload_needs_transfer_fix` and the thread-send `size_computable`.

Both are one shared `flatness_walk` with a `mode`, so the structural arms exist
once and cannot drift. They differ in exactly one leaf: `ParameterType::Res(_)`
is memcpy-copyable and not arena-transferable.

**The trap:** classifying a site by "it has 'arena' in the name" is wrong.
`copy_value_to_current_arena` takes the *memcpy* predicate — most of its callers
are in-arena (the `Result` wrap is reached by any `TRAP`), and asking the arena
question there changed codegen for a fixture containing no threads at all.
