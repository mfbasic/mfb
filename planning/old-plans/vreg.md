# x86-64 non-vreg register audit (plan-00-H)

## RESOLUTION 2026-07-01 (final): ALL register patching DELETED — GPR and FP

The vreg cleanup is **fully done**. No register rename/patch pass exists in the
compiler anymore; `run_register_allocation` receives a stream that carries only
virtual registers, ABI-role registers, and pinned registers. Byte identity was
explicitly retired by the user, so this was executed as a real codegen-changing
refactor gated on **behavioral** acceptance (the `.run`/exit oracle) plus the ULP
harness.

**Stage 1 — GPR scratch (~4,300 sites, 19 builder files):** every hardcoded
`x8`–`x17`/`x20`–`x28` string became a vreg minted at the emit site
(`temporary_vreg()`), one per distinct scratch register per method. Cross-method
"survivor register" contracts were made explicit parameters
(`emit_string_split_write_entry` source-data base; the simd-clamp scalar tail).
Stale `mark_register_used("xN")` calls removed.

**Stage 2 — high-FP kernel file (205 sites, 3 kernel files):** the SIMD/
transcendental kernels' function-global `v16`–`v31` coefficient file became
**explicitly threaded FP vregs** (`temporary_fp_vreg()`):
- `builder_simd_float_math.rs`: a `KernelRegs` struct (16 FP vregs, field names
  keep the historical `v16`..`v31` spelling) minted per kernel invocation in the
  four `lower_simd_float_*` entries and threaded through the whole emitter call
  graph (setup → bodies → error reduce → horner).
- `builder_pow.rs`: `PowHomes` became lifetime-generic; its sixteen high homes
  are minted per `emit_pow_scalar` invocation (the `d3`–`d7` input homes stay
  physical).
- `builder_simd_fixed_math.rs`: the sqrt kernel's `one`/`neg_mask`/`mask`/`sel`
  are minted in the lowering and threaded as parameters.

**Infrastructure that made Stage 2 real (not just renamed):**
- AArch64 `FP_ALLOCATABLE` now includes `d16`–`d31` (caller-saved first, the
  callee-saved `d8`–`d15` last), since the kernels no longer own the bank.
- FP spills are 128-bit `str q`/`ldr q` into 16-byte slots on AArch64 (mirroring
  x86 `movups`) — a 64-bit `str d` would drop a spilled vector's high lane. The
  spill base, slot stride, and `finalize_frame` save-area shift are all
  16-aligned; `ldr_q`/`str_q` gained the GPR-scratch address fallback (+ sizing)
  for offsets past the 65520-byte scaled ceiling.
- The AArch64 vector-operand parser accepts `dN` (the allocator's FP names; the
  op defines the arrangement).
- Width contract (same as the historical physical bank): vector-valued FP vregs
  are never live across a *returning* call — kernels' only in-interval `bl`s are
  dead-end error returns. Documented on `KernelRegs`.

**Deleted:** `vregify_scratch_registers` → `vregify_kernel_fp_registers` → GONE;
`Backend::vregify_high_fp` and `Backend::pins_closure_env_register` hooks removed.

**Verification (all post-deletion):**
- AArch64 full acceptance: **976 tests, ZERO behavioral regressions** (10
  codegen-snapshot goldens regenerated — frame-layout churn from the 16-byte
  slot stride).
- ULP harness (`runtime_ulp.py`): exp/asin/acos/atan2/pow 100% ≤1 ULP, fmod
  maxULP=0; tan's two 2-ULP-vs-macOS points and log/log10's subnormal-input
  misses are **bit-identical to the pre-change compiler** (verified against a
  HEAD-baseline build — pre-existing kernel characteristics, macOS tan itself is
  >1 ULP off truth there).
- x86-64: 128 kernel/float/vector/string tests cross-built and run on BOTH boxes
  — Alpine musl (2227) **128/128** and Ubuntu glibc (2228, musl loader via a
  user-namespace bind mount; the box lacks `/lib/ld-musl-x86_64.so.1` without
  root) **128/128** — stdout + exit codes bit-exact vs the AArch64 goldens.
- Rust unit tests: 4 failures are pre-existing on HEAD (stale x86 encode tests +
  a docs test), unrelated.

--- historical audit below ---

## STATUS 2026-07-01: plan-00-H COMPLETE — console + thread + tls surface identical to AArch64

The whole console/runtime feature surface builds and runs on linux-x86_64 with
output byte-identical to the AArch64 oracle (976/976 acceptance; full box sweep
532/538 runnable — the 6 non-passes are environmental/test-portability only, see
note-01 + the sweep notes, NOT codegen). tls (OpenSSL dlopen) and http are now
enabled and live-verified; sqlite3 dlopen works on the box. thread.* works.

The x86 remap turned out to be the real spine, not per-helper vreg migration:
"get the ABI mapping + libc imports right and the shared helpers just work." The
N rows below were NOT individually vreg-converted; they were made correct by the
remap fixes (staged-result RETS coloring, rsp offset shift, 7/8-arg convention,
map-bucket sizing, ABI-regs-out-of-builder-scratch) and are box-verified working.
So the "N" column now means "still names hardcoded scratch" (a COSMETIC cleanup
toward the end goal), NOT "unverified / broken".

The ONE remaining architecture difference: **-app (GUI app mode) is not wired for
linux-x86_64** (`supports_app_mode() == false`). App mode is GTK4/glibc-only even
on AArch64; the x86 GTK backend is the only unbuilt feature. Console is complete.

--- original audit below (historical; the N/Y column predates the remap-era) ---

# x86-64 non-vreg register audit (plan-00-H)

Master list of builder emit/lower functions that name registers **outside** the
vregify pool (`x8`–`x17` / `x20`–`x28`) — i.e. the `x0`–`x7` ABI range, held
result registers, and pinned regs. These are the functions the
`vregify_scratch_registers` pass does **not** auto-cover, so each must be
validated (or converted) by hand for x86-64.

## What "Verified" means here (strict)

**Y** = I ran a program that exercises this function **on the x86-64 box** and its
output matched AArch64 (the oracle) — including at least one non-trivial/edge
input (multibyte, chained, etc.), **not** a single happy-path value.

**N** = anything less: crashes, wrong output, can't build on x86, only ever run
on one easy input, or never run at all. If it isn't proven, it's N.


## Verified working (Y) — the whole list

| Function | Verified working (Y/N) | Evidence |
|---|---|---|
| `lower_string_concat` | Y | box runs of `upper(a)&lower(b)`→`ABcd`, chained `a&"-"&a&"-"&a`→`x-x-x`, `"len="&toString(byteLen(s))`→`len=5` |
| `emit_utf8_decode_next` | Y | produce byte-identical output to AArch64 on the box |
| `emit_utf8_encode_next` | Y | produce byte-identical output to AArch64 on the box |
| `emit_utf8_encoded_width` | Y | produce byte-identical output to AArch64 on the box |
| `emit_unicode_u32_mapping_lookup` | Y | produce byte-identical output to AArch64 on the box |
| `emit_unicode_property_lookup` | Y | produce byte-identical output to AArch64 on the box |


## Converted to vreg

> **What Y/N mean (goal = delete the `vregify_scratch_registers` patch):** a function
> is **converted (Y)** when it mints its scratch as vregs (`allocate_register`/
> `temporary_vreg`) and names NO hardcoded `x8`–`x17`/`x20`–`x28`. **N = still names
> hardcoded scratch** — it may RUN correctly on x86 (the vregify patch renames its
> scratch at the seam), but the patch can't be deleted until it is converted.
> "Works on the box" ≠ "converted." NOTE: making a helper *work* by moving its
> off-pool ABI scratch (x0–x7) INTO the patch range (e.g. `emit_parse_decimal_string_to_double`,
> `emit_double_overflow_check` → x20–x24 this session) is the OPPOSITE of converting —
> those stay **N**.

| Function | Verified working after convert (Y/N) |
|---|---|
| `lower_string_concat` | Y |
| `emit_utf8_decode_next` | Y |
| `emit_utf8_encode_next` | Y |
| `emit_utf8_encoded_width` | Y |
| `emit_unicode_u32_mapping_lookup` | Y |
| `emit_unicode_property_lookup` | Y |
| `emit_integer_to_string_value` | Y |
| `lower_strings_repeat` | Y |
| `emit_collection_data_pointer` | Y |
| `emit_load_collection_payload` | Y |
| `lower_map_probe_helper` | Y |
| `lower_map_build_buckets_helper` | Y |
| `lower_map_bucket_put_helper` | Y |
| `emit_map_query_key` | Y |
| `emit_compare_bytes_branch` | Y |
| `emit_comparable_values_match_branch_from_slots` | Y |
| `emit_collection_payload_matches_value_branch` | Y |
| `emit_collection_payloads_match_branch` | Y |
| `lower_strings_join` | Y (`200db306`) |
| `emit_string_to_int_value_base` | Y (`200db306`) |
| `lower_strings_pad` | Y (`200db306`) |
| `emit_chars_set_contains_branch` | Y (`200db306`) |

## Not verified (N)

| Function | Verified working (Y/N) |
|---|---|
| `emit_string_to_int_value` | N |
| `emit_float_to_string_value` | N |
| `emit_build_error_loc` | N |
| `emit_error_register_return` | N |
| `emit_allocation_error_return` | N |
| `lower_build_error_loc` | N |
| `lower_make_error_result` | N |
| `emit_utf8proc_sequence_init` | N |
| `emit_utf8proc_decode_next` | N |
| `emit_hangul_composition_attempt` | N |
| `emit_string_byte_range_equal_branch` | N |
| `emit_unicode_whitespace_branch` | N |
| `lower_strings_split` | N (still names x10–x17/x20–x21) |
| `lower_strings_normalize_nfc` | N |
| `emit_double_overflow_check` | N (moved x5–x7 → hardcoded x20–x22; works, not converted) |
| `emit_parse_decimal_string_to_double` | N (moved x0–x7 → hardcoded x20–x24; works, not converted) |
| `lower_simd_binary` | N |
| `lower_simd_clamp` | N |
| `emit_simd_clamp_scalar` | N |
| `emit_float_to_int_overflow_to_err` | N |
| `lower_fs_atomic_write_helper` | N |
| `lower_fs_create_temp_file_helper` | N |
| `lower_fs_read_bytes_path_helper` | N |
| `lower_fs_read_text_path_helper` | N |
| `lower_fs_write_bytes_path_helper` | N |
| `lower_fs_write_text_path_helper` | N |
| `lower_fs_open_helper` | N |
| `lower_fs_eof_helper` | N |
| `lower_fs_read_all_bytes_helper` | N |
| `lower_fs_read_all_helper` | N |
| `lower_fs_read_line_helper` | N |
| `lower_fs_write_all_bytes_helper` | N |
| `lower_fs_write_all_helper` | N |
| `lower_io_poll_input_helper` | N |
| `lower_io_read_byte_helper` | N |
| `lower_io_read_char_helper` | N |
| `lower_io_read_line_helper` | N |
| `emit_configure_stdin_terminal` | N |
| `emit_restore_stdin_terminal` | N |
| `lower_thread_start_helper` | N |
| `simple_thread_handle_helper` | N |
| `thread_queue_read_helper` | N |
| `thread_queue_write_helper` | N |
| `emit_set_color` | N |
| `emit_terminal_size` | N |

## Conversion status & end goal

END GOAL: **when x86-64 is done there is no hardcoded scratch register left
anywhere, and the `vregify_scratch_registers` patch pass can be deleted.** Rule:
any N function touched for x86 work must be fully vreg-converted before it counts
as done.

- **Fully vreg-clean + committed:** `lower_string_concat` (`b36995da`+`5f316bf2`),
  all `unicode.rs` `emit_*` (`0b2e3fff`, `4fc69fed`, `5f316bf2`),
  `emit_integer_to_string_value` (already vreg).
- **Vreg-clean, still N (verification-blocked):** `emit_string_to_int_value` —
  valid inputs match the oracle on the box (pos/neg/zero/i64-max), but its error
  paths route through the unverified error emitters; it is Y for valid input only,
  so it stays N until the error cluster is verified.
- **Not converted:** everything else on the N list.

conversion ≠ verification: a converted function is only Y once it runs on the box
and matches the AArch64 oracle.

## Error-handling cluster (blocks many N rows)

`emit_error_register_return` → `_mfb_make_error_result` (`lower_make_error_result`)
and `emit_build_error_loc` → `_mfb_build_error_loc` (`lower_build_error_loc`) are
the shared error path. Until they are verified on x86, every function with an
error branch (toInt invalid/overflow, index-out-of-range, TRAP, …) can only be
Y for its non-error path. Verifying + converting this cluster unblocks a large
fraction of the N list.

## Collection/list ops — WORKING on x86

List get/iterate, map get/set/iterate, and find/contains/filter/append over
Integer/String/record lists all match the AArch64 oracle on the box. The shared
collection layer (`emit_collection_data_pointer`, `emit_load_collection_payload`,
the map hash helpers, the comparison helpers) is vreg-converted and verified.

## split/join — RESOLVED (historical)

split/join now run correctly on the box (verified vs oracle). The root cause was
the ABI-role heuristic collapsing values onto `rax`, since fixed by the
`map_scratch_register` pool reorder + rax/rdx exclusion + the incoming-param and
call-boundary mapping fixes in `remap_x86_abi`. Original notes kept below.

`lower_strings_split`/`lower_strings_join` still SIGSEGV on x86. Their out-of-pool
x2-x6 scratch IS now vreg-converted (aarch64 acceptance 975/975), and the list
pointer is carried in a vreg — but the fault persists. Root cause is NOT scratch:
these functions are unusually register-heavy, and the MIR confirms the copy loop
uses distinct vregs (%v26/%v30-34). On x86's 5-register allocatable budget the
x86 linear-scan **collapses several simultaneously-live vregs onto rax** (a
non-allocatable register): the copy loop's count/src/byte all become rax
(`movzbq (%rax),%rax` with rax=byte value). This is an x86 register-model /
linear-scan spill bug under high pressure, exposed only by these two functions.

Evidence (x86 `-ncode` dump of the byte-copy loop): the whole loop uses only
`rax` and `r10` — `mov rax,r10` (twice), `ldr_u8 rax,[rax]`, `str_u8 rax,[r10]`,
`add_imm rax,rax,1`, `sub_imm rax,rax,1` — so src pointer, counter, and byte all
alias `rax`. **rax is NOT in the allocatable set** {r10,r11,rbx,r12,r13}, and the
linear-scan's spill-scratch selection (`linear_scan.rs` ~L195-240) only ever
picks from `allocatable`, so rax does NOT come from the reload path. Its origin is
still unconfirmed — needs a real trace of neutral-MIR → `select_x86` → x86
linear-scan for THIS loop to see whether a physical `x9`/`x0` leaks through
`select_x86` (x9→rax via map_scratch, x0→rax as result) or a vreg is mis-colored.
Start there before touching the allocator. Candidate structural fixes once the
origin is known: widen the budget (free arena_base from r15 per plan-00-H §7;
make rcx/r8/r9 allocatable with call-clobber spilling). NOT a scratch task.

## x86-64 status snapshot — POST-LIBC-PIVOT (current)

The plan-00-H **libc-linking pivot** superseded most of the old remaining list:
x86 now links libc dynamically (PLT/GOT/interp like AArch64) and the entry
delegates to the shared `lower_program_entry`. Split/join, float output, and
uncaught-error print (below) all WORK now — kept as history in the "Fixed" and
"Superseded" notes. The vreg-migration-of-helpers approach is largely replaced by
"get the ABI mapping + libc imports right and the shared helpers just work."

**Working on x86 (box-verified vs AArch64 oracle):** integers incl. negatives;
strings (concat, case, split/join, pad, repeat, trim, replace, contains/starts/
ends, byteLen, toInt base 2-36); lists (get/iterate/find/contains/filter/append/
sort); maps (get/set/iterate); records; **full error handling incl. uncaught-error
top-level `Code:/Message:` print**; **Float** (arith, sqrt/abs, IEEE/NaN compares,
toFloat(String), toString(Float), round/ceil/floor/toFixed); **bits** (clz/ctz/
popcount/bswap/rotate/rbit); **runtime helpers** datetime/io/fs/term/net via libc;
**closures/lambdas** (forEach/reduce); **SIMD `vector::`** (dot/length/min/max/abs/
distance/normalize/types) + light-spill transcendentals (atan2/acos/log10) via
FP-register-allocated SSE2/SSE4.1/FMA3.

**Also fixed since (committed):**
- **datetime/json/regex copy-record crash** — call-result `x1`(→rdx) used later as a
  copy-loop dest collapsed to rax (the OK tag=0) → null-dst store. Fix = `map_abi_register`
  None-fallback returns `RETS[n]` not always rax (`f33509f2`). Fixed datetime×3 + json.
- **closure/lambda garbage-env crash** — `x28`=`CLOSURE_ENV_REGISTER` was in the vregify range;
  excluded (x86-gated via `Backend::pins_closure_env_register`, `344a4c41`). AArch64 975/975.
- **SIMD FP register allocation** — vregify now renames physical `v16`-`v31`→FP vregs (x86-gated,
  `Backend::vregify_high_fp`, `e0843cd6`); FMA3 (fmla/fmls), fcvtzs_v/scvtf_v (lane-serial), sshr_v,
  **128-bit FP spills** (`str_q`+16-byte slots, `78a6d8cc`). Basic `vector::` (dot/length/min/…) +
  light transcendentals (atan2/acos/log10) WORK on the box.

**Remaining — all RESOLVED (this list is historical; see the RESOLUTION header):**
1. ~~Heavy-spill kernel + intervening-call corruption~~ — FIXED: root cause was the
   FP-accumulator read-modify-write spill (`65666cae`) + `bit_v`/`math_pool_base`
   (`d5e70921`/`726ed2dc`), not evict-slot/frame layout. Math/vector suite green.
2. ~~createTempFile/resource + regex SIGSEGV~~ — FIXED via the frame-offset shift
   (`39c4bcd8`) and thread stack size (`72782fd5`); regex + resource tests pass.
3. ~~thread.* / tls.* unimplemented~~ — DONE: thread trampoline (`b5a10f76`), tls
   OpenSSL dlopen enabled + live Google test (`f2096497`).
4. Deferred SIMD ops `fcvtas_v`/`sshl_v`/`ushl_v` are unused by the green suite;
   `frinta_v` is implemented (`e0512433`). The only architecture-level gap left is
   **-app (GUI) mode**, which is GTK4/glibc-only and tracked separately.

## Fixed

- **Negative integers on x86** (was: `toString(-1000)` → `-`). Root cause was NOT
  the emitter — it was the x86 encoder lowering `sub d, xzr, r` (the AArch64
  negate idiom) to `sub d,d` = 0. Fixed in `alu3`/`enc_neg` (commit `2da7fd4b`);
  fixes negation everywhere (toString, toInt, unary minus, fixed-math, bits).
- **3-operand mul/alu aliasing** (`beafad25`): `mov dst,lhs; OP dst,rhs`
  miscompiled when `dst==rhs` (`index*entry_size`→`index²`, corrupting collection
  addresses → SIGSEGV). Fixed via commutative reorder / neg+add.
- **Collection element addressing** (`72b9251d`): `emit_collection_data_pointer`
  (x6/x7) + `emit_load_collection_payload` (x3/x4/x5) vreg-converted → FOR EACH
  iteration + list get correct.
- **Map get/set** (this session): the three map hash helpers
  (`build_buckets`/`probe`/`bucket_put`) homed their incoming x0/x1/x2 params in
  rax/rdx (via the Ret-role fallback), which their own `div`/`msub` clobber, and
  probe's params also had to survive the nested `build_buckets` call. Fixed by
  capturing each param into a vreg at helper entry (allocator keeps it safe /
  spills across the call). Verified: list+map get, map set, map iteration.

