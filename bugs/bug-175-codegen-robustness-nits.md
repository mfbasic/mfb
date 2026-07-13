# bug-175 — shared-codegen robustness nits (unchecked size arithmetic, latent register/alignment gaps, dead branches, stale comments)

Last updated: 2026-07-12
Severity: LOW (batch) — all latent or documentation/robustness.
Class: Correctness / Memory-safety (latent) / Dead-code.
Status: Open

## Findings

**A. `money::round` re-multiply can overflow i64 near the Money range boundary.**
`src/target/shared/code/builder_money.rs:124-126`. `result = rounded * divisor`
uses `abi::multiply_registers` (no trap), and `emit_apply_rounding` can return
`q+1`, so `(q+1)*divisor` can exceed `i64::MAX` for a near-max Money — wraps to a
large negative Money. The "cannot overflow" comment is wrong. Fix: use
`emit_checked_integer_multiply` (trap ErrOverflow).

**B. `strings.split`/`case_map` use unchecked arena-size arithmetic (and a
count/write width mismatch on malformed UTF-8).**
`src/target/shared/code/builder_strings_builtins.rs:1551` (split size via plain
`multiply/add`, unlike graphemes/to_bytes/nfc/replace/join which use
`emit_checked_size_*`; split's `count` is the most expansion-prone) and `:507`
(case_map `+9` header). Also `:484` — `case_map` length pass counts the *original*
decode width while the write pass emits the *re-encoded* width (U+FFFD width 1 vs
3), so a malformed input under-allocates and the OOB is only *detected* by
`emit_write_cursor_assert` (:567), not prevented (NFC recomputes width and stays
consistent). Both gated by the well-formed-UTF-8 ingress invariant (unreachable in
practice). Fix: route split/case_map sizes through the checked helpers; size
case_map's count pass from the re-encoded width like NFC.

**C. `builder_values` dead Const-String branch + latent union-size mismatch.**
`src/target/shared/code/builder_values.rs:218` — the `type_ == "String"` branch in
the `NirValue::Const` arm is unreachable (`static_string_value` returns Some for
Const Strings first, :207). `:1004` — the `Constructor` union-size computation
skips resource variants (via `union_variant_fields`) while `UnionWrap` (:1111)
counts a resource variant as 1 word, so a union mixing resource+data variants
would allocate different block sizes on the two paths (latent; only if mixed
unions are permitted). Fix: drop the dead branch; make Constructor size account for
resource-variant width like UnionWrap (or assert no resource variants).

**D. `emit_compare_bytes_branch` destructively advances the caller's key
register.** `src/target/shared/code/builder_collection_compare.rs:29-31` (from the
inline arms at :250-259, :343-352). It does `add_immediate(right, right, 1)` on the
caller's key register per matched byte and never restores it, so after a
non-first-entry compare the key pointer is advanced. Latent: every byte-compared
type is currently excluded as a map key (unions/lists non-comparable; records take
the stack-slot arm). Fix: advance private scratch copies of left/right.

**E. append/insert/prepend skip `list_element_padding_alignment`.**
`src/target/shared/code/builder_collection_mutate.rs:638-651, 703-709, 1107-1112,
1747-1752`. Mutation paths concatenate payloads at the raw running `dataLength`
(not padded to 8), unlike the literal writer
(`builder_collection_layout.rs:1211-1214, 1443-1448`). For a
`List OF <record-with-inline-String>` etc. an inserted element can start on a
non-8 offset. Values stay correct on aarch64/x86 (unaligned loads allowed); the
exposure is an unaligned-access fault/perf hit on strict-alignment rv64. Fix:
align the in-place append/prepend/insert data placement like the literal writer.

**F. `type_utils` naive `") AS "` split mis-parses higher-order function types.**
`src/target/shared/code/type_utils.rs:264-278`. `callable_return_type` uses
`rsplit_once(") AS ")` and `function_type_parts` uses `split_once(") AS ")`, so a
function returning/accepting a function type splits at the wrong `") AS "`. The
`function_type_parts` caller fails safe (clean Err); `callable_return_type` can
pick the innermost return, but both a function-value and Boolean are 8-byte words,
limiting damage. Fix: paren-depth-aware scan for the top-level `") AS "`.

**G. `term_grid` present buffer has no trailing-escape headroom.**
`src/target/shared/code/term_grid.rs:56` (sizing) / `:990-1021`. The out buffer is
sized exactly `rows*cols*OUTBUF_PER_CELL` (last region in the arena block); the
fixed ~24-byte trailing reset/CUP/cursor sequence is appended after the per-cell
loop with no reserved slack, so a near-saturating repaint (pathological geometry)
writes past the block. Latent. Fix: add `TRAILER_SLACK` (+64) to the out-buffer
size.

**H. Stale comments / dead labels / duplicate constants (batched, dead-code).**
`builder_simd_float_math.rs:1065-1066` (comment names callee-saved v8/v9 as
scratch though code uses VEC_SCRATCH[0]/v31 — a future edit trusting it would
corrupt callee-saved regs); `builder_simd_math.rs:18-19` (`FIXED_ONE_MINUS_1_STR`/
`FIXED_FRACTION_MASK_STR` both == `"4294967295"`); `code/term.rs:623-625`
(`emit_clear_grid` docstring says "current background" but zero-fills to black —
correct behavior, misleading doc); `entry_and_arena.rs:888` (comment claims large
non-exact chunks are "recovered by the large flush-before-grow drain" — no such
drain exists; large bins reclaimed only at `arena_destroy`); `src/doc.rs:255`
(dead no-op `let _ = &mut package_name;`).

**I. Entry emits a branch to a label only defined for `returns == "Integer"`.**
`src/target/shared/code/entry_and_arena.rs:363`. The `else` branch (:356-365)
emits `branch_hi("entry_exit_range_error")` while the handler block (:367-380) is
gated on `language_entry_returns == "Integer"`; the two live under different `if`
conditions. Unreachable today because `validate_entry_point`
(`src/manifest/entry.rs:38`) forces a FUNC entry to `Integer` and a SUB to
`Nothing`, but a future entry return type (e.g. `Byte`) would branch to an
undefined label → assembler/link failure. Fix: assert `language_entry_returns` is
`Integer`/`Nothing` at the top of `lower_program_entry`, or emit the handler under
the same `else`.
