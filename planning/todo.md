Cleaned up codegen

- = Not reviewed
+ = Started
@ = Reviewed

[@] app
[@] astrings
[@] audio
[@] bits
[@] collections
[@] crypto
[@] csv
[@] datetime
[@] encoding
[@] errorcode
[@] fs
[@] http
[@] io
[@] json
[@] math
[@] money
[@] net
[@] os
[@] process
[@] regex
[@] strings
[@] term
[-] thread
[@] tls
[-] vector

---

# Cleanup investigation (2026-08-23, read-only survey)

## Q1 — After tls migration, can the deprecated registry items be removed cleanly?

**No. tls is irrelevant to them — it's already fully migrated.**

- tls is already on the clean-room registry: every member (`func_connect/listen/accept/
  read/write/poll/close`) uses `Body::native_os_seam` + the generic `native::lower_tls_helper`
  dispatcher (`src/codegen/builtins/tls/native/mod.rs:365`). It calls none of the deprecated
  shims (its only reference to one is inside a code comment).
- The 7 deprecated items live in `src/codegen/registry/mod.rs` and are doc-comment markers
  (`/// #[deprecated(note="migrate registry()...")]`), NOT real `#[deprecated]` attributes —
  so the build does not warn on them. The real migration is an API-shape change: route callers
  through the `registry()` accessor instead of these `Box::leak`-ing free-function shims.
- Each still has a live NON-tls production caller (removal is blocked on repointing these):
  - `call_return_type` (:1849)            → `src/builtins/mod.rs:351`
  - `rewrite_target` (:2243)              → `src/ir/lower.rs:2986`
  - `argument_types` (:2505)              → `src/builtins/mod.rs:423`
  - `call_param_names` (:2629)            → `src/builtins/mod.rs:653`
  - `call_param_name_overloads` (:2667)   → `src/builtins/mod.rs:625`
  - `default_argument_padding` (:2694)    → `src/ir/lower.rs:2805`
  - `resource_close_function` (:2227)     → NO production caller; only `#[cfg(test)]` refs in
    audio/os/process. Closest to dead. (Naming trap: distinct from the still-live
    `builtins::resource_close_function` wrapper → `resource::builtin_resource_close_function`.)
- Separate, also-not-tls deprecation markers gated on crypto/strings/collections/vector
  SOURCE-GENERICS work: `codegen/builtins/encoding/mod.rs:235`, `collections/mod.rs:215`,
  `vector/mod.rs:80`.

## Q2 — Any remaining hardcoded registers, or all moved to vreg?

**The compiler's own codegen path is fully vreg. Leftovers are only in hand-written platform
emitters (tracked as bug-387).**

- Clean: neutral instruction stream, all per-arch code plans (`linux_aarch64/linux_riscv64/
  linux_x86_64/win_x86_64` `{code,plan}.rs`), shared lowering. Flows as vregs (`Operand::vreg`),
  typed ABI tokens (`Operand::abi`), or `%`-sentinels; realized to physicals ONLY at two
  legitimate seams: `src/target/shared/abi.rs` `realize_abi_token` (:381-443) and
  `src/arch/*/select.rs` + `regmodel.rs`. (x86/riscv backends carry no x86/riscv literals —
  they remap the AArch64-spelled neutral stream.)
- Leftovers run BELOW the register allocator, so their target is neutral ABI tokens
  (`LOCAL`/`SCRATCH`/`FP_SCRATCH`/`c_arg`), NOT vregs. `Asm` helpers already accept
  `impl Into<Operand>`, so raw `"xNN"` strings are pure leftover:
  - `src/target/linux_gtk/term_draw.rs` — LARGEST gap, ~110+ literals (callee-saved x19-x28,
    scratch x9-x17, FP d0-d3) intermixed with tokens; clearly mid-conversion.
  - `src/target/linux_gtk/bootstrap.rs` — ~27 lines (x9/x10/x11/x13/x19, two raw sp).
  - `src/target/macos_aarch64/app/{bootstrap.rs,term_view.rs,mod.rs}` — only C-arg staging
    x0-x3; callee-saved bank already migrated. Low risk.
  - `src/target/macos_aarch64/tls.rs` — x1/x2 in the arg-reg→context-offset table.

## Q3 — Move ParameterType to an integer enum from IR downward?

**Yes, startable — but the enum already exists; the task is pushing its boundary UP, and the
right model is string-interning inside the existing structural enum, NOT a flat integer enum.**

- `ParameterType` (`src/types.rs:22`) is already a structural enum and the internal currency of
  the codegen registry. String-based today: IR (`src/ir/*`), monomorph (`src/monomorph/*`), and
  the registry's own boundary, which round-trips string → `parse` → unify/substitute → `name()`
  → string per call (`ParameterType::parse` @ `src/types.rs:146`, 70 call sites).
- Measured hotspots this eliminates (non-test greps): 111 scalar-name string `==`; 218
  structural prefix matches (`strip_prefix("List OF ")` etc.); 698 `type_:/returns: …to_string()`
  allocation sites; 49 `format!("List OF …")`; 17 `IrValue` variants each carrying
  `type_: String` (52 alloc sites in `src/ir/lower.rs` alone); 675 `.type_` accesses.
- NOT a flat enum: records/unions/user types (`Named`) and generics (`Var`) are open sets and
  types nest (`List OF Foo`). Correct model = structural enum with an interned handle at the
  nominal/var leaves. Precedent: `binary_repr` interns type names into a `type_id` table
  (`src/binary_repr/builder.rs:82`).
- BLOCKER to fix first: interning is currently `Box::leak` (`src/types.rs:216`) — fine at the
  low-frequency registry boundary, but leaks per-IR-node if pushed down. Replace with a real
  interner returning `Copy Symbol(u32)`/`TypeId` (also makes Named/Var compares integer compares
  and the enum cheap to clone).
- Recommended start order:
  1. Interner → `Copy Symbol`; Named/Var hold Symbol; keep parse/name. (Localized to
     `src/types.rs` + call sites.)
  2. Convert registry boundary to pass/return `ParameterType`, shrinking the 1137-line
     `resolve_call` string matcher (`src/codegen/builtins/general/mod.rs:287`).
  3. Flip IR `type_`/`returns`/`kind` (`src/ir/types.rs`, `src/ir/value.rs`) String→ParameterType,
     converting ONCE at cut point `ir::lower_augmented_project` (`src/ir/lower.rs`); update
     `ir::verify`, `binary_repr`, codegen; keep `name()` only at serialize seams
     (`src/ir/binary.rs`, `src/ir/json.rs` — wire format stays string for ABI stability).
  4. (Later, separate phase) Give monomorph a typed representation.
- CAVEAT: monomorph runs on the AST BEFORE IR (`src/cli/build/mod.rs:332` precedes `:416`) with a
  parallel string type system (`src/monomorph/helpers.rs:41,171`). An IR-only cut leaves monomorph
  string-based → does NOT speed up monomorphization. If ever unified onto `ParameterType`,
  reconcile `MapEntry OF`/`Result OF` first (monomorph models them structurally; `parse` doesn't).

## Q4 — What other areas are a mess? (ranked next cleanup targets)

1. **syntaxcheck vs ir::verify — two overlapping semantic-check passes.** Documented,
   half-finished migration (`src/rules/mod.rs:5-11`, plan-20-Z): "not-yet-relocated" vs
   "relocated" rules. Mirrored filenames (resources/types/link ↔). Rule codes: 58 in
   syntaxcheck vs 118 in ir/verify — actively moving, neither empty. ~19k lines, duplicated
   traversal, goldens pinned to transitional ordering. Finish relocation + delete syntaxcheck
   half = single biggest structural simplification.
2. **Three hand-written app/terminal runtimes, no shared layer** (overlaps Q2/bug-387). Codegen
   targets already unified via `target/linux_common/` (bug-321), but app runtimes were not:
   `macos_aarch64/app/` (8,002 LOC), `win_x86_64/app/` (3,318), `linux_gtk/` (4,813). Terminal
   render + app_io + bootstrap triplicated; plan-13/94/98 keep adding each feature 3×. ~16k LOC.
3. **CLI monoliths + stringly-typed errors.** `cli/build/mod.rs` (3,581), `cli/pkg.rs` (3,296).
   `Result<_, String>` at 484 sites (cli/manifest/resolver/os). Three error mechanisms coexist:
   `rules`+`PendingDiagnostic`, `ast::DocError`, raw `Result<_, String>`. Consolidate tooling side.
4. **os/ per-OS object writers/linkers + dead prototype.** Three stacks (linux 4,233 / windows
   3,500 / macos 3,013) over a thin shared seam; partly inherent (ELF/Mach-O/PE differ). Quick
   win: delete `src/os/windows/link/spike.rs` (426 LOC proof-of-concept PE that writes
   `mfb_spike_proof.txt`, sitting in the linker path).
5. **Hand-rolled JSON serializers.** serde avoided (2 files); ~27 files hand-emit JSON
   (`ir/json.rs` 908, `nir/json.rs` 1,096, `audit/json.rs` 639, …). `src/json.rs` is only a
   shared escaper/parser — no shared value→JSON writer. Mechanical consolidation.
6. **(Diagnose first — likely intentional) src/ir vs target/shared/nir.** Two IR layers; NIR used
   by all backends + ~40 codegen files. Probably deliberate layering (IR → NIR → arch encoders);
   confirm it earns its keep before growing either.

Cross-cutting symptoms (evidence, not targets): `#[allow(clippy::too_many_arguments)]` ×117
(missing context structs, in target/os/syntaxcheck); `#[allow(dead_code)]` ×34 (mostly still in
codegen). TODO/FIXME grep understates debt — this team encodes it in `planning/`/`bug-NN` docs.

Suggested order: #1 and #2 lead (largest, actively worsening). #3 and #5 contained mechanical
wins. #4 spike.rs is a quick delete. #6 diagnose-first.
