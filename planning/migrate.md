# Migrating a builtin package to the clean-room `Body::abi_function` shape

This is the playbook for taking one `src/codegen/builtins/<pkg>/` package from
the legacy shape (`Body::native_os_seam` / pre-finalized "hatch" adapters /
standalone `_mfb_rt_<pkg>_app_*` helpers reached by `bl`) to the **crypto shape**:
one clean-room `Body::abi_function` lowering per member, app-mode sequences
appended inline, shared logic in `gen_*` seams. `crypto` and `io` are the two
reference migrations — read one of them alongside this doc.

Read first: `.ai/codegen-invariants.md`, `.ai/arch-abi.md`,
`.ai/resources-packages.md`, `.ai/testing-gates.md`. This doc is the *procedure*;
those are the *invariants*.

---

## 0. The end state (what "done" looks like)

Per member `<name>`:

- `func_<name>.rs` — the descriptor (`register`), the authored docs (`INTRO` /
  `DESC` / `EX`), and **one** `lower_<name>(builder, _args, ctx) -> Result<ValueResult, String>`
  registered as `Body::abi_function(lower_<name>)`. The lowering emits its vreg
  body directly into `builder.instructions` / `builder.relocations` and sets
  `builder.stack_size`; the `abi_function` wrapper finalizes it.
- No `lower_<name>` + `emit_<name>_body` split. One function.
- Shared-by-multiple-members logic lives in `gen_<something>.rs`
  (multi-member lowering seams) or `helper_<something>.rs` (leaf helpers), **never**
  imported `func_ -> func_`.
- App mode appends the platform sequence in place (`ctx.platform.emit_app_<pkg>_<op>(…)`),
  no standalone helper, no `bl` hop, no `native_os_seam`, no `hatch_finalized`.

The `mod.rs` module list stays alphabetical: `func_*`, then `gen_*`, then `helper_*`.

---

## 1. The `abi_function` finalizer contract (the part that bites)

The wrapper runs the body through `finalize_vreg_body_with_locals` (see
`src/codegen/engine/util/vreg_frame.rs`). It **panics** ("zero-physical-register
invariant") on any operand that renders to a physical register. So:

- **Never name a raw register** (`"x0"`, `"x9"`, `"rax"`, `"rsp"` offsets aside).
  Use the role tokens from `src/target/shared/abi.rs`:
  - `abi::mfb_arg(n)` / `abi::mfb_return(n)` — the MFB calling convention.
  - `abi::c_arg(n)` / `abi::c_return(n)` — a C/libc call's args/result.
  - `abi::return_register()` = `mfb_return(0)`.
  - `abi::SCRATCH[n]` — caller-saved scratch pool (machine-floor, realized below
    the allocator). Fine for values **not** live across a call.
  - `abi::ARENA` (`"arena_base"`, pinned x19/equiv) — the per-thread arena-state
    base. Read package/term state off this, not off a repurposed callee-saved reg.
  - `abi::ZERO`, `abi::stack_pointer()`, `abi::link_register()`.
- **Values live across a call must be allocator vregs**, not `abi::LOCAL[n]`.
  `LOCAL` realizes to a callee-saved register but the finalizer does **not** save
  it — it will be clobbered across the call. Use `Vregs::new()` + `vregs.next()`
  (`%vN`); the allocator colors them callee-saved *and* the frame saves them. (This
  was the plan-101 flush correctness bug.)
- **No manual frame.** Do not emit `abi::subtract_stack` / `add_stack` / a
  `label("entry")` in an append/abi_function body. The `CodeBuilder` is pre-seeded
  with the entry label; the finalizer emits the prologue/epilogue, saves `lr`, and
  reserves the callee-saved area. Just `push(abi::return_())` at the end.
- **Stack scratch you take the address of** (a buffer a syscall fills, a byte you
  `write`): set `builder.stack_size = N` and address it at `abi::stack_pointer() + off`
  for `off` in `[0, N)`. The finalizer places its spills *above* `N`, so the two
  never overlap. (See `func_read_byte`'s 208-byte frame.)
- **Win64 calls with >4 arguments:** never hardcode the outgoing stack slots at
  `sp+0x20`/`sp+0x28`. Stage them with `abi::outgoing_stack_arg_store(src, k)`
  (k=0 → 5th arg, k=1 → 6th, …). `finalize_frame` sizes the 32-byte shadow region
  + the outgoing tail and resolves the sentinel to `rsp + shadow + k*8`. The
  finalizer also auto-reserves the shadow space + call padding + 16-alignment for
  *any* body that makes a call, so a Win64 append body needs no manual frame math.

---

## 2. `AbiCtx` — what the lowering gets

`ctx` (`src/codegen/registry/mod.rs`) carries the platform-dependent seam:

- `ctx.platform: &dyn CodegenPlatform` — the per-target emitter (OS syscalls, app
  hooks). `ctx.platform_imports: &HashMap<String,String>` — the flavor-bound import
  map to thread into `platform.emit_*`.
- `ctx.build_mode.is_app()` — console vs `--app`.
- `ctx.term_state_offset` / `ctx.presentation_mode_offset` — arena offsets for
  `term::`/app-mode state (`Option`, `None` when the program never uses them).

Platform OS seams (read/write/poll/isatty/etc.) live on the `CodegenPlatform`
trait (`src/codegen/engine/types/types.rs`) with per-target impls under
`src/target/<target>/code.rs`. `types.rs` holds the **default** (`None` / stub);
the real body is the override. The stub returning `None` is how an unported target
(e.g. riscv app mode) reports "unsupported" — the member turns that into
`app_unsupported(ctx.platform)`.

---

## 3. App-mode: decouple → append shape

Legacy app mode emitted a standalone `_mfb_rt_<pkg>_app_<op>` helper (a finalized
`AppHookBody` = `(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>)`) and the
member `bl`'d it. Kill that:

1. **Reshape the `CodegenPlatform` hook** from returning an `AppHookBody` to the
   **append shape**, mirroring `emit_is_terminal`:
   ```rust
   fn emit_app_<pkg>_<op>(&self, symbol: &str, /* params */,
       instructions: &mut Vec<CodeInstruction>,
       relocations: &mut Vec<CodeRelocation>) -> Option<Result<(), String>> { None }
   ```
   Update every per-target impl in `src/target/*/code.rs`.
2. **Convert each platform body** to append/vreg-clean form (§1). The body pushes
   into the passed `instructions`/`relocations` and returns; no own frame.
3. **Member side:**
   ```rust
   if ctx.build_mode.is_app() {
       ctx.platform.emit_app_<pkg>_<op>(&symbol, /* … */,
           &mut builder.instructions, &mut builder.relocations)
           .ok_or_else(|| super::app_unsupported(ctx.platform))??;
   } else { /* console vreg body */ }
   ```
4. **Delete** the standalone-helper emission block (search `builder/mod.rs` for the
   `module.build_mode.is_app()` push loop) and the `<PKG>_APP_*_SYMBOL` constants in
   `error_constants.rs`.

### Converting a standalone (`AppHookBody`) body — the traps

Standalone app bodies are token-*realized* but **not** vreg-finalized, so the
originals legitimately use raw physical registers, `abi::LOCAL[n]`, and a manual
frame. When you inline one into a vreg-finalized member body, all of that becomes
illegal (§1). Two recurring gotchas:

- **Shared asm helpers** (`build_nsstring_from_cstring`, `emit_present_needs_display`,
  GTK `load_state`/`store_state`/`state_array`, the term-active gate) are called
  from *both* still-standalone bodies (bootstrap, `term::` setters) **and** the new
  vreg bodies. A raw `x0`/`x9` in them is fine standalone but panics once finalized.
  Fix by spelling them through a role token that *realizes to the same physical
  register* (`SCRATCH[0]` ≡ x9), keeping the standalone native goldens byte-identical
  — or make a `_vreg` twin (as plan-101 did for `emit_present_needs_display`) if the
  standalone spelling must not move.
- **Objc/GTK/Win32 arg staging**: on aarch64 `mfb_arg(n)` and `c_arg(n)` coincide,
  but on Win64 they do **not** — use `c_arg`/`mfb_arg` deliberately per the callee's
  convention (a libc/OS call is `c_arg`; an internal `_mfb_rt_*` call is `mfb_arg`).

---

## 4. Collapse single-use emitters

Any `emit_<name>_body(symbol, imports, platform, …) -> (Vec, Vec, usize)` that has
exactly one caller (`grep` it — count the call sites, ignore doc-comment mentions)
folds into its `lower_<name>`: bind `let symbol: &str = &builder.current_symbol.clone();`
(and `platform`/`platform_imports`/`app_mode` from `ctx`), keep the body verbatim,
and replace the trailing `Ok((instructions, relocations, FRAME_SIZE))` with
`builder.instructions.extend(instructions); builder.relocations.extend(relocations);
builder.stack_size = FRAME_SIZE; Ok(ValueResult { … })`. Byte-identical.

For a large body (hundreds of lines) move it with `sed -n 'A,Bp'` byte-exact rather
than retyping, then fix only the signature + tail with `Edit`.

---

## 5. Shared seams → `gen_*` / `helper_*`

If two or more `func_*` members import the same item, it must **not** live in a
`func_*` file. Move it:

- A shared **member lowering** (e.g. one body serving `print`/`write`/`printError`/
  `writeError`, selected by flags) → `gen_<family>.rs`, `pub(crate)`.
- Shared **primitives** (e.g. the stdin byte/UTF-8 readers used by
  readByte/readChar/readLine) → `gen_<area>.rs` or `helper_<x>.rs`.

`io` ended with `gen_is_terminal`, `gen_write_family`, `gen_read_family`
(primitives), `gen_read_line_family`. Moving code between files is pure motion →
byte-identical; the acceptance gate proves it.

---

## 6. Symbol-family preservation (byte-identity of runtime symbols)

`abi_function` members are wrapped once into a `_mfb_rt_<pkg>_<pkg>_<member>` helper.
Keep the package's runtime symbol family so goldens/links stay byte-identical: see
`abi_function_family` in `src/target/shared/runtime/mod.rs` (it maps the member back
to its owning package family instead of the generic `Abi` family). A concrete
package also needs its `IMPL_NAMES` table kept in sync in `ir`/`lower`
(see `.ai/resources-packages.md`, memory "Package rewrite paths").

---

## 7. Verification gate (run ALL of these)

1. `rustup run 1.96.0 cargo build --bin mfb` → **0 warnings** (trim unused imports
   the moves leave behind).
2. `rustup run 1.96.0 cargo test --bin mfb` → unit tests (includes the
   codegen-inspection tests; a stale hardcoded offset there is usually the test,
   not you — dump the `.ncode` first, see memory).
3. `rustup run 1.96.0 cargo test --test <cli_*app*>` → the app-mode integration
   tests that actually build+link a bundle (the real macOS runtime proof).
4. **Acceptance byte-identity** (NOT in `cargo test`): build **release** first
   (`cargo build --release --bin mfb` — the harness runs `target/release/mfb`), then
   `bash scripts/test-accept.sh target/release/mfb /tmp/accept-out` (2nd arg is an
   `rm -rf` scratch dir — never a real path). **Only the goldens you intended to
   change may differ.** A diff anywhere else is a bug you introduced — objdump/dump
   that one fixture before concluding.
5. For the intended (non-byte-identical) goldens: review the diff, then
   `bash scripts/sync-goldens.sh target/release/mfb '<fixture-glob>'` and re-run (4)
   to confirm green.
6. **Cross-target codegen** (surfaces finalizer panics for platforms you can't run
   locally): for a project hitting every member,
   `target/release/mfb build -app -target {linux-aarch64,linux-x86_64,windows-x86_64} -ncode <proj>`.
   A finalizer panic fails the build here.
7. **Remote runtime** for anything you changed on GTK/Windows: ship + run on the
   boxes in `.ai/remote_systems.md` (GTK 2226/2228, Win11 2230). Codegen-clean +
   byte-preserved logic is strong but not a runtime proof.
8. `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
   (the root `--all` does not reach the `repository/` path dep).

---

## 8. Cleanup traps that fail the gate

- **Spec citations** `[[src/…:symbol]]` in `src/docs/spec/**` dangle after a rename
  — repoint them (the `spec_citations_resolve` golden checks the symbol exists).
- **Unsupported-platform stub tests** (riscv app mode) and any unit test that calls
  the old signature (`emit_app_*_helper`) break `cargo test` even though `--bin`
  compiles — the two are separate compile targets, so build `--tests` too.
- **Deleted error-constant symbols** must be removed everywhere; grep before delete.
- **`git diff --stat`** before trusting any delegated edit (memory: subagent edits
  can silently vanish).
- Header `//!` doc comments and inline mentions of the old function names — update
  them; they read as lies otherwise.

---

## 9. Reference commits

- `crypto` migration — the original template (search `git log` for the crypto
  cleanup).
- `io` migration — plan-101; the app decouple→append conversion, the GTK `x9`
  latent-panic fix, the Win64 outgoing-arg-sentinel conversion, and the full
  `func_/gen_` split. Single squashed commit: `io: app-mode io off the decouple +
  collapse to crypto shape (plan-101)`.
