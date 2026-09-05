# Per-architecture ABI & codegen invariants

Load-bearing per-architecture ABI, register, and codegen facts for the MFB native backends, grouped by target.

## x86-64 (SysV)

### x86 native-call return uses c_return, not the aligned bank

An external C call through `blr` (a LINK thunk, or any raw `branch_link_register` to a C function) returns its integer/pointer result in the **C-ABI return register** = `abi::c_return(0)` = `rax`. It does NOT land in the aligned MFB result bank: on x86-64 `abi::return_register()` (`mfb_return(0)`) realizes to `call_args[0]`, which is `rdi` on SysV and `rcx` on Win64 (see `realize_abi_operand` in `src/arch/x86_64/select.rs`: `(Mfb, _) => call_args`, `(C, Ret) => C_RETS=[rax,rdx]`). Nothing stages `rax` into the aligned bank after a raw `blr`, so capturing `return_register()` reads a caller-saved register the callee clobbered — every native return comes back as garbage/0.

AArch64 and RISC-V (and macOS) hide this: there `return_register()` and `c_return(0)` are the SAME physical register (`x0`/`a0`). So a LINK/native-call result path that "works on macOS aarch64" can be silently broken on both x86-64 ABIs. This class is invisible to the acceptance oracle because x86-64 LINK *execution* is not recorded there (only macOS aarch64 runs) — it only shows up in an on-box run (found on boxes 2230 Win11 and 2227 Alpine musl x86_64).

Rule: when a shared-codegen site consumes the result of an external/C call, name `abi::c_return(0)`, never `abi::return_register()`. The reverse also holds for staging C-call *arguments* — `emit_arena_map`-style code moves `c_return(0)` into `return_register()` explicitly after a Win64 call because the two differ.

**Win64 skips even the direct-`bl` staging (bug-452).** On linux-x86_64 a *direct* external `bl` (`emit_external_call` → `emit_linux_c_call`) stages `mov rdi,rax` right after the call, so `return_register()` reads are correct there and ONLY a raw `blr` is unstaged. On Win64 that staging is gated off (`emit_linux_c_call` runs its `mov` only for `target == "linux-x86_64"`; `win_x86_64::code::emit_external_call` passes `"windows-x86_64"` to skip it), so BOTH a direct IAT `bl` AND a COM-vtable `blr` leave their result in `rax`, never `rcx`. A stale comment in `win_x86_64/code.rs` claiming "Win64's MFB result bank is rax-based" is WRONG — `mfb_return(0)` realizes to `rcx` (`call_args[0]`). So on Win64 every external result read (`ole_call`/`com_call`'s `sxtw`, any HRESULT/DWORD check) must come from `c_return(0)`; a hand-emitted backend that sign-extends the result in place (`sxtw rcx,rcx`) sign-extends the stale first arg. Fix shape byte-identical on AArch64: `sxtw(return_register(), c_return(0))` (a funnel — read `rax`, re-home into the aligned bank so downstream reads stay correct), which is `sxtw x0,x0` on AArch64/RISC-V.

**The thread runtime is a THIRD emission path, and it was unstaged (found via a red CI `rt_native_size_arith_overflow::thread_queue_limit_in_range_accepted`).** `emit_thread_external_call` (`src/codegen/runtime/thread/runtime_helpers.rs`) does NOT go through `emit_external_call`/`emit_linux_c_call` — it pushes its own `abi::branch_link(symbol)` + `external_branch(...)` reloc — so it never got linux-x86_64's `mov rdi,rax` staging, and on Windows the arms hand-synthesize the POSIX return contract themselves. Every shared caller therefore had to read `abi::c_return(0)`; they read `abi::mfb_return(0)` instead, so on BOTH x86 ABIs `pthread_create`, `pthread_mutex_init`, `pthread_cond_init`, `pthread_cond_timedwait` and `nanosleep` results were taken from a clobbered caller-saved register. Effect: `thread::start` raised `ErrInterrupted` on every x86-64 program (45 of 46 `tests/rt-behavior/threads` fixtures failed on box 2228; on Win64 the `SleepConditionVariableSRW` BOOL read from `rcx` made every timed wait look like a timeout). AArch64/RISC-V were byte-identical and green throughout, and the host-only acceptance run on macOS could not see it — the artifact gate flagged exactly 3 `.ncodesum` (thread/io × linux-x86_64/windows-x86_64), which is the ONLY on-Mac signal this class produces. **When auditing this rule, enumerate emission paths, not call sites: any code that pushes `branch_link` + an external reloc itself bypasses the staging.**

Blast-radius note: a fixture that `IMPORT`s `tls`/`audio` transitively embeds those backends' `_mfb_rt_*` bodies (e.g. `http` imports `tls` via `http::serverSSL`), so a tls/audio codegen change drifts that fixture's linux-x86_64/win64 `.ncodesum` too — the byte-identity gate catches it; regenerate the importer's golden as well.

### The program entry needs the +8 call parity when it is CALLED (app mode)

`finalize_frame` adds `frame_call_padding()` (8 on x86-64, 0 on AArch64/RISC-V) to
any frame whose function makes calls, so that a callee entered at `rsp % 16 == 8`
reaches its own call sites at `rsp % 16 == 0`. **The program entry never passes
through `finalize_frame`** — `entry.rs` builds its own frame — so it never got that
bias.

For an ordinary program that is correct: the kernel enters `_start` at
`rsp % 16 == 0` with **no return address pushed**, so a 16-multiple frame is right.
In **app mode** the entry is a called function — the worker shim calls it under
`MACAPP_PROGRAM_SYMBOL` — so it arrives at `rsp % 16 == 8`, and without the bias
every call beneath it, for the whole program, is misaligned. Gate on
`entry_called_as_function`.

**The symptom is not where the bug is.** A misaligned stack only faults when a
callee uses an aligned SSE access, so it surfaces as a crash deep inside libc:
`__libc_calloc` under `g_idle_add`, on the first `app::setMode`, in **Console** mode
as much as Canvas. That reads as heap corruption and is not. The faulting
instruction was `movaps %xmm0,(%rsp)` — an *aligned* store. **Disassemble the
faulting instruction before believing a malloc frame**; `objdump -d
--start-address=…` on the libc offset from the core is enough, and it is the
difference between "something scribbled on the heap" and "rsp is 8 off".

Walking it the rest of the way needs the core: `coredumpctl dump`, then gdb's
`find /g $rsp, $rsp+N, <return-address>` to locate each frame's pushed return
address, which gives that frame's `rsp` at its own `call`. Comparing that against
the function's `sub rsp,imm` says whether the frame is wrong or its caller is.

Windows x86-64 has the same `frame_call_padding()`, so the fix moves its app-mode
`.ncodesum` goldens too; macOS AArch64's is 0 and does not move.

### `c_arg(6)`/`c_arg(7)` are NOT arguments on SysV, and staging into them is the damage

`abi::c_arg(n)` is slot `n` of the **aligned call bank**, which is longer than the ABI's
register-argument list: SysV `CALL_ARGS` is `[rdi, rsi, rdx, rcx, r8, r9, rax, rbp]` but
only the first six carry arguments (bug-296; the register model says so —
`X86SysVRegisterModel::external_int_argument_registers() == 6`). So a hand-written emitter
that stages an 8-argument C call entirely in `c_arg(0..8)` puts argument 7 in `rax`,
argument 8 in **`rbp`** — the frame pointer, which the allocator excludes precisely because
it is one — and the callee reads two stack slots nothing wrote. AArch64 and riscv64 pass
eight in registers, so the identical code is correct there: **this is invisible on a Mac
host and on every AArch64 box.**

Spilling *after* staging does not fix it. `emit_external_int_call` gets away with
`outgoing_stack_arg_store(c_arg(n), …)` only because the indices it spills on Win64 land on
caller-saved `rdi`/`rsi`; at index 7 the staging write itself has already destroyed `rbp`.
For an overflow argument, **write the value straight to the outgoing area** and never touch
`c_arg(n)` for `n >= external_int_argument_registers()` — see
`runtime/canvas/vulkan.rs:emit_int_arg_zero`, which branches on the register model so the
bytes are unchanged wherever the target's registers cover the call.

Symptom when it happens: every API call reports success and the frame comes back blank.

### An INTERNAL 8-argument call stages `rbp` legitimately — every foreign boundary must save it

The section above is about a C call, where `rbp` at index 7 is a mistake. For an **MFB-to-MFB**
call it is the convention: SysV has six argument registers and the internal convention needs
eight, so `CALL_ARGS` extends with `rax` and `rbp` (bug-296). Caller and callee agree, so
nothing is wrong *inside* MFB code — and the arity that triggers it is not exotic.
`__canvas_geoDistance` takes 22 parameters, and `__canvas_drawGeometry` stages `rbp` for it
six times in one function:

```
mov r8,r10 / mov r9,r10 / mov rax,r10 / mov rbp,r10
bl _mfb_ifn_canvas_5FgeoDistance
```

`rbp` is **callee-saved under SysV**, so the moment MFB code is entered from foreign code,
that staging destroys the caller's frame pointer. **Every boundary a non-MFB caller enters
through must save and restore `rbp`**: the thread trampolines, and each `_mfb_gtkapp_*`
callback GTK invokes. The GTK callbacks always did (one manual `str_u64 rbp` and its load
apiece); `_mfb_rt_canvas_graphics_entry` did not, and returned to glibc's `start_thread` with
`rbp = 0x404e000000000000` — the double `60.0`, the `radius` of a circle in the scene —
whereupon `start_thread` ran `mov -0x98(%rbp),%rax` and took SIGBUS.

Two things that make this expensive to find:

* **The frame's callee-saved set cannot see it.** `calleeSaved` is computed from the
  registers the ALLOCATOR assigned; an ABI-staged `rbp` was never allocated, so
  `__canvas_drawGeometry` records `["r12","r14","lr"]` and saves `rbp` nowhere. The
  mechanism that should catch this structurally is blind to it by construction.
* **It is invisible off x86-64 and host-dependent on it.** AArch64 passes eight in registers
  (`x7`, caller-saved) so macOS and the aarch64 rows are clean, and on x86-64 whether the
  clobbered `rbp` is ever *dereferenced* depends on the libc's own path after the start
  routine returns — it faults on ubuntu-24.04 runners and not on Debian 13, Alpine, or a
  container with the same GTK. It cost 68 of ~90 canvas tests, 55 SIGBUS and 13 SIGSEGV,
  the two alternating as one wild address lands in different places.

The safety condition is **not** "keep functions under 8 parameters" — canvas passed that long
ago. It is "a new foreign-boundary entry point saves `rbp`". Count entry points, not
parameters.

### `RESULT_VALUE_REGISTER` is `rsi` on SysV, and so is `SCRATCH[1]`

The aliasing above is not only about *arguments*. `RESULT_VALUE_REGISTER` is
`abi::mfb_return(1)`, and `%retMFB` draws from the same aligned bank, so on SysV it is
`rsi` — which `map_scratch_register` also hands to `SCRATCH[1]`, `SCRATCH[11]` and
`LOCAL[2]`. `RESULT_TAG_REGISTER` (`mfb_return(0)`) is `rdi`, shared with `SCRATCH[2]`,
`SCRATCH[12]` and `LOCAL[3]`.

The deadly shape is writing the result *between* two uses of the aliasing token:

    cmp  rsi, 0 ; je build      ; SCRATCH[1] = a tri-state
    mov  rsi, 1                 ; RESULT_VALUE_REGISTER = TRUE — overwrites it
    cmp  rsi, 1 ; je done       ; compares the answer with itself: always taken

`canvas::vulkanReady` did exactly this and answered TRUE on a machine with no Vulkan
driver, from its second call onward. On AArch64 the two are `x1` and `x10`, so it is
correct there — invisible on a Mac host, again.

**Rule: finish every comparison before writing the result, and carry the compared value
in a `builder.temporary_vreg()` rather than a fixed `SCRATCH[k]`.** The full SysV map,
worth having in front of you when hand-staging:

    rdi  SCRATCH[2]  SCRATCH[12] LOCAL[3] c_arg(0)/mfb_return(0)
    rsi  SCRATCH[1]  SCRATCH[11] LOCAL[2] c_arg(1)/mfb_return(1)
    rdx                                   c_arg(2)/mfb_return(2) c_return(1)
    rcx  SCRATCH[9]                       c_arg(3)/mfb_return(3)
    r8   SCRATCH[3]  SCRATCH[13] LOCAL[4] c_arg(4)/mfb_return(4)
    r9   SCRATCH[4]  SCRATCH[14] LOCAL[5] c_arg(5)/mfb_return(5)
    rax                                   c_arg(6)/mfb_return(6) c_return(0)
    rbp              LOCAL[0]             c_arg(7)/mfb_return(7)
    rbx  SCRATCH[0]  SCRATCH[10] LOCAL[1]
    r10  SCRATCH[5]  SCRATCH[15] LOCAL[6]
    r11  SCRATCH[6]  SCRATCH[16] LOCAL[7]
    r12  SCRATCH[7]  SCRATCH[17] LOCAL[8]
    r13  SCRATCH[8]  SCRATCH[18] LOCAL[9]

### The x86 scratch pool aliases the SysV argument bank — mid-staging, that corrupts an argument

`map_scratch_register` folds `x9`–`x30` onto an 11-entry pool: `SCRATCH[1]`→`rsi`,
`SCRATCH[2]`→`rdi`, `SCRATCH[3]`→**`r8`**, `SCRATCH[4]`→`r9`, `SCRATCH[9]`→`rcx`. Those are
`c_arg(1)`, `c_arg(0)`, `c_arg(4)`, `c_arg(5)` and `c_arg(3)`. On AArch64 the two banks are
disjoint (`x9+` vs `x0`–`x7`), so a helper that uses a fixed `SCRATCH[k]` as a temporary is
correct there and silently overwrites an already-staged argument on x86-64.

The trap needs three ingredients, which is why it is rare and why it bites hard: a helper
with a fixed scratch, called *between* two argument stagings, on a call with enough
arguments to reach the aliased index. `canvas`'s `emit_state_load` used `SCRATCH[3]`, so
`vkCmdBindDescriptorSets` received the graphics-state block pointer as its
`descriptorSetCount` and walked a one-element array for a dozen entries.

**Rule: a helper that may be called between argument stagings takes its temporary from
`builder.temporary_vreg()`, never a fixed `SCRATCH[k]`.** The allocator then cannot pick a
live argument register. (Same rule, different reason, as calling through a resolved function
pointer.)

Vulkan validation layers are the fastest way to see this class on Linux — they are not
installed on the test boxes but `apt-get download vulkan-validationlayers` + `dpkg -x` into
a scratch dir works as a plain user, then
`VK_LAYER_PATH=<dir>/usr/share/vulkan/explicit_layer.d
LD_LIBRARY_PATH=<dir>/usr/lib/x86_64-linux-gnu
VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`.

### Never mix a raw `xN` with its role token in one x86 app body

`finalize_x86_app_function` (`src/target/linux_gtk/mod.rs`) renames each **distinct
token string** to its own vreg. `%scratch0` realizes to `x9`, so a body that loads
into raw `"x9"` and compares `abi::SCRATCH[0]` uses one register on AArch64 and
**two** on x86-64 — the compare then tests an uninitialized register. The GTK finish
helper did exactly this on its "is there a transcript" test, so a headless run took
the GUI arm and formatted into a chunk it had not allocated.

**The enforced rule is stronger than "pick one spelling per value": there are now no raw
`xN` register strings in `src/target/linux_gtk/` at all**, and
`no_emitter_spells_a_register_as_a_raw_register_string` keeps it that way.

**The token spelling is not only about correctness — it is what keeps the line visible
to the guard.** `shared_lowering_names_no_physical_register` decides whether a line is
an emission context by looking for `abi::` on it, so a line that spells *everything*
raw has no `abi::` and is skipped entirely. `term_draw.rs` had
`emit_cell_dim_to_d(&mut asm, "d0", "x22", …)` — a forbidden FP register sitting beside
a forbidden GPR, invisible because neither was a token. Removing the raw GPR is what
turned the guard red on the `"d0"` that had been there all along. A guard whose reach
depends on a convention shrinks silently every time the convention is broken, so a
green run is evidence about the guard's reach and not only about the code.

(The FP half is pure visibility: converting the nine `"dN"` literals to
`abi::FP_SCRATCH[n]` leaves **both** linux-aarch64 and linux-x86_64 byte-identical,
because `finalize_x86_app_function` renames the integer scratch/parking space and the
FP bank passes straight through. Only the GPR rewrite moved bytes, and only on x86.) The finish
helper was not the only site — a census found sixteen functions mixing, including
`emit_term_resize_helper`, which loaded the cell width into `"x10"` and divided by
`abi::SCRATCH[1]`. Rewriting all 146 sites to tokens left the linux-aarch64 binary
byte-identical (the spellings already named one register there) and changed the x86-64
one, which is what the fix *is*. "Mixed" needs a def/use analysis; "present" is a
substring search, which is why the test asserts the stronger form.

### remap_x86_abi: linear vs CFG

`remap_x86_abi` (`src/arch/x86_64/select.rs`) resolves residual `x0`–`x8` operands to SysV homes by ABI role. Its incoming-parameter test used flags advanced in **emitted order** (`boundary_since_entry` / `defined_since_entry`), so any block *branched into from before* a call inherited that call's state because the call was emitted first. `fs::setBuffered` is exactly that shape: its enable arm reads the `File*` parameter but sits below the disable arm's `bl _mfb_rt_fs_file_drain`, so the parameter was colored by its next boundary (`ret`) into `rax` instead of `rdi` → store through garbage → SIGSEGV on every x86-64 program that enabled per-file buffering. Fixed via a forward MUST dataflow: `entry_clean` + `entry_undef`, meeting by intersection.

**The reusable lesson:** this pass is a pile of heuristics over emitted order — `boundary_before`, `next_after`, `staged_result_def`, `defined_since_boundary`, `block_entry`. Several ARE CFG-aware; the parameter pair was not. When an x86-only miscompile has no aarch64/rv64 twin, suspect this pass first, and check whether the specific fact is tracked linearly or along control flow.

**How it was proven, cheaply:** `mfb build --target linux-x86_64 --ncode` on a 4-line repro, then `git show HEAD:src/arch/x86_64/select.rs > …`, rebuild, dump again, diff the one helper. `base=rax` vs `base=rdi` in one line. Do this before reading the pass — it is ~700 lines of dataflow and reading it does not converge.

### remap_x86_abi is a pure rename

`remap_x86_abi` (`src/arch/x86_64/select.rs`) is effectively a **pure per-operand register rename** on the current corpus. Its ONLY `instructions.insert(...)` is the param-bridge prologue (`for (k, home) in &param_home { … if home == arg { continue } … }`), and that prologue fires **0 times** across the whole 1162-fixture corpus on BOTH linux-x86_64 and windows-x86_64 — because `param_home[n]` is only ever set to `call_args[n]` (the arg reg), making `home == arg` always true. Measured via a `BUG387-PARAMBRIDGE` probe under `MFB_BUG387_AUDIT`.

**Why it matters:** the operand-level `MFB_BUG387_AUDIT` cross-check only measures per-operand register divergence, NOT inserted instructions — so "audit at zero ⟹ byte-identical deletion" would be UNSOUND if the fixpoint inserted anything the direct map can't reproduce. It doesn't. This validates the premise of deleting the fixpoint and means **no Category-2 value exists** (a pure rename gives every value one consistent register, else the current passing code would miscompile without the absent bridge). Any deletion of the fixpoint should still re-confirm param-bridge=0 immediately before deleting (a future param layout with `param_home != arg` would revive it).

Separately: the redundant `mov %ret1,%ret1` self-moves in emitted code (486 linux-aarch64 / 234 macos / 486 riscv, symmetric with x86 `mov rdx,rdx`) come from `move_register(RESULT_VALUE_REGISTER, abi::RET[1])` where `RESULT_VALUE_REGISTER == RET[1]` (`error_constants.rs:26`), at 19 shared-builder sites — byte-baseline, non-divergent, do NOT blanket-elide them.

### bug387 divergence audit is blind to Category 2

The `MFB_BUG387_AUDIT` cross-check (`src/arch/x86_64/select.rs` `map_token_direct` vs the `remap_x86_abi` fixpoint) emits `BUG387-MISMATCH` only where a deferred role token maps context-free to a DIFFERENT register than the CFG inference chose. That population is **entirely Category 1** (the token is the wrong role alias; re-tokenize the producer to the inferred role → byte-identical on x86 AND aarch64/riscv). **Category 2** — a genuine call RESULT reused as an ARG needing a `mov rdi,rax` on x86 / `mov x0,x0` no-op (elision) on aarch64 — is **invisible to this audit by construction**: an explicit staging `mov %argK,%retK` has both operands AGREE (no mismatch), and same-index physical reuse is staged below the token layer. So a "0 Category-2 sites" audit result is NOT evidence Category 2 is empty — it must be measured separately (enumerate emitted same-register moves / values needing two tokens). See `planning/plan-71-census.md`. Dominant idiom: `%ret0`→rdi (result-named value used as call arg 0), ~99.7% of raw divergences, in the shared arena/string/collection/record builders. Every inferred register has a role-token preimage, so there is no residue category.

### Linux GTK app-entry helpers must use typed `Operand::Abi`, not raw `xN`

`src/target/linux_gtk/{bootstrap,term_draw,app_io}.rs` + the libc-start trampoline in `mod.rs` are hand-authored **AArch64** assembly (raw register string operands) transpiled to x86 by `select_x86` via `finalize_x86_app_function`. On AArch64 `x0` is BOTH C-argument-0 AND the C return register; on x86-64 SysV they are DIFFERENT (`rdi` vs `rax`). plan-85-D deleted the `remap_x86_abi` CFG role fixpoint these bodies relied on, so a raw `"x0"` now maps context-free to the call bank (`rdi`) — silently breaking every `x0` that holds a C-call RESULT (e.g. `gtk_application_new`'s return read from `x0` → invalid `GtkApplication`; the linux-x86_64 GTK app was broken in release Aug 8→the fix). RULE for these bodies: **every `x0`–`x8` is a typed `abi::Operand::Abi` token, never a raw register string** — `abi::c_arg(N)` for a C-call argument, `abi::c_return(0)` for a C-call result OR a C callback / `GSourceFunc` return value (a `gboolean`/`int` returned to GTK/GLib/libc: `_main`, the pthread worker, `key_pressed`, `window_closed`, the `*_idle` handlers). MFB-internal (`call_internal`) results/returns stay `c_arg(0)` — the aligned MFB convention puts result==arg on SysV. Where a C-call RESULT feeds the NEXT call's arg0 with no intervening write (`gtk_scrolled_window_new()`→`g_object_ref_sink()`), insert an explicit `abi::move_register(abi::c_arg(0), abi::c_return(0))` (a real `mov rdi,rax` on x86; a `mov x0,x0` no-op on AArch64) — the old `stage_result_reuse_x86` did this with a bare `mov x0,x0` that relied on the deleted role remap and is now removed. The x86 residual-`x0`–`x8` `debug_assert` in `select.rs` is the live guard: a raw `xN` left in these bodies reds a debug `-target linux-x86_64 -app` build. Validate a change by running the built app on box 2227 (`scripts/test-appimage.sh <exe> --box 2227 --libc musl`) — "started the inner GTK program" = the app object is valid; the GObject-CRITICAL "invalid non-instantiatable type" is the miscompile signature.

### glibc/musl thread-entry alignment

**Truth:** every x86-64 thread library reaches the start-routine (the trampoline `lower_thread_trampoline` in `src/target/shared/code/runtime_helpers.rs`) with a `call` — glibc `start_thread` → `pd->start_routine(...)`, musl's pthread dispatch, Windows `BaseThreadInitThunk` — so the trampoline is ALWAYS entered at `sp % 16 == 8` and needs exactly ONE +8 realign → an **88-byte frame** on both libcs and Windows. The realign gates on `platform.arch() == "x86_64"` ALONE (not `libc()`/`family()`). aarch64/riscv64 use `bl`/`jal` (link register, sp unchanged), enter `sp%16==0`, keep the 80-byte frame. macOS (aarch64) no realign. (An earlier belief that glibc entered 16-aligned was WRONG.)

**Box proof (deterministic, 5/5 each):** glibc 2228 — 88-byte frame runs, 80-byte SIGSEGVs; musl 2227 — 88-byte runs, 96-byte (double realign) SIGSEGVs. Fixture: `tests/rt-behavior/threads/thread-bounded-queues` (expected `one two three alpha beta gamma`). A misaligned worker faults on the first SSE-to-stack-local (`movaps`/`movdqa` in `fstatat`/`pthread_create`/ntdll SwitchBack).

**How the wrong belief hid for so long:** an earlier fix gated glibc OUT of the shared realign (frame 80), but `linux_x86_64::emit_thread_trampoline` carried a SECOND unconditional +8 override that silently restored glibc to 88 (correct by accident) and pushed musl to a broken 96. The override was deleted and the shared gate made per-arch. The old "80 runs on glibc" proof must have measured a build still carrying the override (effective 88) — its stated fixture never actually ran an 80-byte glibc frame. Guard test: `target::linux_common::code::tests::thread_trampoline_x86_frame_is_88_on_both_libcs`.

**Enduring lesson:** an x86-64 thread/stack-ABI fix proven on ONE libc is NOT proven — verify glibc (2228) AND musl (2227). And distrust a doc/memory's alignment claim until box-run. musl binaries are dynamically linked to `/lib/ld-musl-x86_64.so.1` (absent on the glibc box), so each libc's binary must run on ITS OWN box; you cannot cross-run a musl binary on 2228 (`mfb build --target linux-x86_64` emits both `-glibc.out` and `-musl.out`).

Also: the Mac's `cargo test` hangs in `macos_tls_write_capacity` (`macos_tls_write_sends_capacity_over_count_byte_list_exactly` runs forever) — environmental TLS-socket hang, unrelated to codegen.

## Win64

### Win64 shadow space + entry ABI

Four Win64-specific codegen invariants proven by getting `-target windows-x86_64` `RETURN 42` to exit 42 on the Win11 box:

1. **Shadow space is REAL and 32 bytes.** Every `call` requires the caller to own 32 bytes ABOVE its `rsp` (`[rsp+8..rsp+40]` in the callee) that the callee may freely clobber. The entry set `arena_state = rsp` (top of frame, nothing below), so every call it made (time/RNG seed, init, main) wrote shadow over `[arena+0]` = the block-list head → `arena_destroy` deref'd garbage → 0xC0000005. Fix: entry reserves `backend.shadow_space_bytes()` at the frame BOTTOM and points the arena register ABOVE it (`sp + shadow`); `shadow==0` on Linux/macOS keeps them byte-identical. See `entry.rs` + `entry_stack_misaligned_on_entry`.

2. **The PE entry is `call`-reached by the loader → `sp % 16 == 8` on arrival**, NOT the Linux `_start` `sp % 16 == 0`. Without one `sub rsp,8` every downstream call is misaligned 8 and the first callee `movaps` faults. Guarded by `entry_stack_misaligned_on_entry()` (false for Linux/macOS).

3. **`return_register()` (rax) ≠ `ARG[0]` (rcx) on Win64.** Linux syscalls read the first arg from x0 = return_register, so shared helpers (e.g. `arena_destroy`) hand the address in `return_register()`. A Windows DLL call (`VirtualFree`) needs it in ARG[0] — insert `move_register(ARG[0], return_register())` first.

4. **The instruction encoder rejects negative immediates** (`move_immediate(_, "-10")` → "invalid immediate '-10'"). Compute negatives via add/sub (e.g. nStdHandle = `-(fd+10)` as `add fd,10; 0 - that`).

The crash was diagnosed WITHOUT a debugger on the box: run the exe, then `Get-WinEvent -ProviderName "Application Error"` gives the fault offset (module-relative); `objdump -d --start-address=IMAGEBASE+offset` lands on the faulting instruction.

### Win64 helper frame + zero-reg traps

Writing a shared `src/target/shared/code/**` helper (numeric `Vregs` + `abi::` builders) that makes a Win64 external call (`emit_libc_call`) with >4 args or any callee at all, two non-obvious traps bite — both invisible on aarch64, both a crash/garbage on x86-64:

**1. Outgoing stack args + call frame must be an explicit `subtract_stack(FRAME)` … `add_stack(FRAME)` bracket, with NO abstract vregs referenced inside it.** `finalize_vreg_body[_with_locals]` runs `finalize_frame`, which SHIFTS every `sp`-relative access UP past the callee-saved area (`adjust_stack_instruction_offsets`). So a store you emit to `sp+0x20` (the 5th Win64 arg slot) lands at `sp+0x20+save_size`, but the callee reads its stack args from the REAL `sp+0x20` → garbage/crash. The shift is skipped only at stack-adjust depth>0 (inside a `subtract_stack`/`add_stack` pair — depth is tracked by counting SubSp/AddSp). Mirror `emit_build_argv_utf8` (win_x86_64/code.rs): reserve ONE frame `subtract_stack(FRAME)` covering shadow `[0,0x20)` + the stack args `[0x20,…)` + any struct locals (STARTUPINFOA/PROCESS_INFORMATION) + your scalar state slots; keep ALL state in those slots and use only `mfb_arg(0..3)` as transient scratch (reloaded from slots after every `emit_alloc`/`emit_libc_call`, which clobber them). No vregs ⇒ no spills that `finalize` would shift out from under the depth-1 accesses. Shadow space is NOT auto-reserved — `call_external`/`emit_libc_call` emit only the `call`; a callee's 32-byte shadow write into `[sp,sp+0x20)` corrupts a frame that didn't reserve it, so EVEN a 2-arg-all-register call (WaitForSingleObject) needs the `[0,0x20)` shadow.

**2. `abi::move_register(reg, abi::ZERO)` does NOT zero a register on x86-64.** There is no hardware zero register; `ZERO` maps to a GPR holding garbage. `store_u64(ZERO, base, off)` special-cases it to an immediate `$0x0` store (so zeroing memory works), but `move_register`/register args do not — the disasm shows `movq %r8,%rcx` (r8 = leftover loop garbage). Zero a register arg with `move_immediate(reg, "Integer", "0")` (the fs Win64 helpers' convention). A CreateProcessA whose `lpApplicationName`/attrs came from `move_register(_,ZERO)` gets a garbage pointer and returns FALSE. aarch64 hides this (xzr).

### Win64 stack growth: a frame bigger than one page MUST probe

Windows does not hand a thread its whole stack. The PE header reserves 8 MiB and
commits 1 MiB (`os/windows/link/pe.rs`); past the committed region the stack
grows **one page at a time**, when an access faults on the single guard page
below it. So a prologue that does a bare `sub rsp, N` for `N > 4096` and then
writes steps clean OVER the guard page into reserved-but-uncommitted memory, and
the OS raises `STATUS_ACCESS_VIOLATION` (`0xC0000005`) instead of growing the
stack. This is the entire reason `__chkstk` exists; MSVC/clang emit it for every
frame over a page.

`finalize_frame` therefore allocates a big frame page-by-page, touching each page,
gated on `Backend::stack_probe_page_bytes()` (4096 on Win64, **0 everywhere
else** — SysV/AAPCS64 grow the stack without cooperation, so they keep the single
`sub sp` and stay byte-identical). The sub-page remainder is left unprobed on
purpose: it can only ever step onto the immediately next page, which is exactly
the single-page step the guard is designed to absorb.

Three things make this bug hard to catch, all of which cost time once:

- **"Frames are small" is false.** `pe.rs` justified the 1 MiB commit with "the
  largest observed is ~9 KiB in `main`". A >4 KiB frame is ordinary: 17 of the 23
  `regex` helpers have one, up to 19688 bytes in `__regex_parseParen`. Check with
  the `sub_sp` imm in a `-ncode` dump before believing any such claim.
- **The 1 MiB commit HIDES it until the stack passes 1 MiB.** Shallow calls with
  huge frames are fine; only recursion reaches the guard. `regex::match` on an
  N-deep group nest costs ≈37.9 KB/level, so it died at N≈36 — ~1 MiB in, i.e.
  exactly the commit size. A crash depth that equals `commit / bytes-per-level`
  is the signature.
- **It is NOT a frame-size anomaly, so don't go tuning frames.** windows-x86_64
  and linux-x86_64 frames agree to within the 32-byte shadow space (parseParen
  19688 vs 19656) and Linux tolerates ~350 levels of the same recursion. Compare
  the two targets' `sub_sp` immediates first; if they match, the stack contract is
  the difference, not codegen bloat.

Byte-identity cannot see this (the bytes are "correct", the missing probe is what
is wrong) and neither macOS nor Linux can reproduce it — execution-verify on the
Win11 box. Note also that `cargo test` fail-fast can hide it for a long time: the
test that catches it (`rt_native_regex_parser_depth`) sorts *after*
`rt_native_io_runtime`, so the windows CI row never reached it while an earlier
binary was red. Use `--no-fail-fast`.

## riscv64

### riscv64 V-extension (RVV) two-profile qemu oracle

Both remote riscv64 boxes LACK the V extension in hardware: `/proc/cpuinfo` isa is `rv64imafdch_...zba_zbb_zbc_zbs_...` — **no `v`** (2229 Alpine musl, 2232 Debian glibc). So a native run only ever exercises the **v=false** (scalar) path.

The **v=true** oracle is `qemu-riscv64` user-mode, which emulates V (and sets `AT_HWCAP` bit 21) even on non-V hardware. No box has it installed and there is no root, but on **2232 (Debian)** you can fetch it without root: `apt-get download qemu-user` → `dpkg -x qemu-user_*.deb ~/qemuroot` → `~/qemuroot/usr/bin/qemu-riscv64` (v10.0.11). qemu-user is Linux-host-only, so it CANNOT run on the Mac; run it on 2232 (riscv-on-riscv user emulation is fine).

Two-profile runtime proof for a linux-riscv64 binary (cross-compiled on the Mac, scp'd to 2232):
- `~/qemuroot/usr/bin/qemu-riscv64 -cpu rv64,v=true  ./bin`  → HWCAP V=1 (0x200000 set)
- `~/qemuroot/usr/bin/qemu-riscv64 -cpu rv64,v=false ./bin`  → V=0 (== a native run)

Verified with a `getauxval(AT_HWCAP)` probe: native hwcap=0x112d (V=0), `v=true` hwcap=0x20112d (V=1). `gcc` (native riscv64) is present on 2232 for building reference probes.

### riscv64 flag-emulation reserved slots

The flagless riscv64 backend (`select_riscv64`) emulates condition flags: a bare (non-fused) `cmp` whose flag-reading branch is not adjacent must keep BOTH compared *values* live from compare to branch. `gp` (x3) holds the lhs. There is **no free second register** for the rhs, so it goes to memory:

- **`tp` (x4) is a silent miscompile.** It is the hardware thread pointer; musl AND glibc pin TLS/`errno` there. Snapshotting into `tp` builds and byte-stabilizes fine and the branch is correct, but the binary SIGSEGVs at runtime the moment control returns to libc. Proven on 2229 (Alpine riscv64). NEVER clobber `tp`. (`gp` is safe — no library uses it after startup.)
- **Shrinking `regmodel::INT_ALLOCATABLE` to reserve a temporary destabilizes the allocator** — see the riscv64 pool-shrink allocator fault below.

So the landed fix snapshots the rhs to a reserved **per-thread memory word** `ARENA_FLAG_RHS_OFFSET`, carved from the rv64-only v128 slot region (`ARENA_V128_SLOTS_SIZE` 128→127 slots, `SLOT_COUNT` 128→127) so `ARENA_STATE_SIZE` and every other target's bytes are unchanged. `store_flag_rhs` spills at the compare, `load_flag_rhs` reloads into `t0` at the branch, addressed off pinned `s11`. Only the label/call invalidations remain (a callee overwrites the shared word; flags never survive a call anyway) — operand-register redefinition is now recoverable. Blast radius is surgical: only functions that hit the bare-compare path (today just `_mfb_rt_sort_string_list`) change bytes; all 6 pre-existing rv64 goldens stayed identical. Lesson: a wrong reserved-slot choice is a runtime-only fault — VALIDATE ON A riscv64 RUN (2229), never a rebuild.

### riscv64 pool-shrink allocator fault (OPEN latent bug)

Removing a single caller-saved temporary (`t3`) from `src/arch/riscv64/regmodel.rs` `INT_ALLOCATABLE` (11→ tried as a fix shape) made **12 unrelated rt-behavior fixtures SIGSEGV on 2229** — fixtures that never touch the flag path (their `.ncode` has no `gp`/`t3`). Restoring `t3` fixed them. So the rv64 linear-scan allocator has a **latent fault that only manifests with a smaller register pool** — a real miscompile, not just more spilling.

**Why:** never treat "shrink the rv64 pool" as a free move; it trips this. The flag-emulation fix was landed WITHOUT touching the pool (memory-word snapshot instead — see the reserved-slots note above).

**How to apply:** this is worth its own bug. To reproduce: drop one reg from `INT_ALLOCATABLE`, rebuild release, build any record/float rt-behavior fixture for linux-riscv64, run on 2229 → SIGSEGV. Suspect the spill-slot / eviction indexing in `regalloc/linear_scan.rs` (off-by-one against pool size).

## macOS AArch64

### macOS codegen latent bugs

The macOS AArch64 backend (`src/target/macos_aarch64/code.rs`, `src/os/macos/link.rs`) had these latent bugs fixed while implementing the filesystem review items.

1. **Raw `open` syscall never detected errors.** `emit_open_file` used a raw `svc` (`DARWIN_SYSCALL_OPEN`). Darwin signals syscall failure via the carry flag and returns the positive errno in `x0`, so `fs::open`/`readText` treated errno (e.g. ENOENT=2) as a valid fd → bogus success or a later seek/read failure. Also `emit_errno` reads libc `___error`, which a raw syscall never sets. Fixed by calling the libSystem `_open` wrapper instead — but `open(path,flags,mode)` is variadic and the Apple AArch64 ABI passes variadic args on the **stack**, so the helper now pushes `x2` (mode) to `[sp]` around the call (`subtract_stack(16); store x2,[sp,0]; bl _open; add_stack(16)`). Without the stack push, write-mode `O_CREAT` opens get a garbage mode and later reopens fail EACCES.

2. **GOT address baked into import stubs used the pre-stub code length.** In `append_import_stubs`, `macho_layout(code_offset, text.len(), ...)` was computed *before* the 12-byte-per-import stubs were appended. The real `data_const_file_offset` (where the GOT lives) is `align(code_offset + final_code_len + data_len, PAGE_SIZE)`. When the stub bytes pushed the total across a 4 KB page boundary, the two `align()` results diverged by a page, so every import stub's `adrp/ldr/br` jumped through a wrong GOT slot → **layout-sensitive SIGBUS with a garbage PC** (a `br x16` to junk). The symptom: a program crashes only at a specific size/register-pressure, and inserting unrelated code (or `io::print` calls that change layout) makes it appear/disappear. Fixed by computing the layout from `text.len() + imports.len() * IMPORT_STUB_SIZE`.

3. **Raw `mmap` in `arena_alloc` ignored the carry flag (same class as #1).** `emit_arena_map` in `src/target/macos_aarch64/code.rs` did a raw mmap `svc`; the shared check (`lower_arena_alloc` in `src/target/shared/code/mod.rs`) only tested `x0 >= 0`. Darwin returns the positive errno in x0 with the carry flag set on failure, so a failed mmap (ENOMEM=12) was treated as a valid mapping and dereferenced → SIGSEGV. Fixed by branching on carry-clear (`b.lo`) for success and otherwise `mvn x0, x31` to a negative sentinel so the shared `b.ge` routes to the OOM path (matches Linux's negative-errno convention). Note the cpsr carry bit is bit 29 (0x2000_0000) — handy to confirm in lldb. Verified by disassembly; a deterministic ENOMEM runtime test isn't portable on macOS because Jetsam SIGKILLs a gradually-growing process before mmap returns ENOMEM. (The `arena_alloc` clobbering x14/x15 bug surfaced this — it requested a single ~72TB mmap, which mmap rejects outright.)

Debugging note: the encoder computes label offsets in a pre-pass via `instruction_size()` then emits separately (`src/arch/aarch64/encode.rs`); a mismatch there would also corrupt branches, but that was verified consistent. Conditional branches use `branch_imm19` (±1 MB, no range check).

### macOS Network.framework async cancel drain

`nw_connection_cancel` / `nw_listener_cancel` transition the object to `cancelled` **asynchronously**: the state-changed handler (STATE_INVOKE) runs later on the shared `mfb.tls` serial queue and dereferences the **arena-allocated** ctx on every invocation. So any codegen path that cancels an nw object and returns immediately leaves a pending handler that runs against a freed ctx once the program exits and the arena is torn down → EXC_BAD_ACCESS, intermittent/load-dependent, macOS aarch64 only.

The fix on every such exit: **drain to the terminal `cancelled` state before returning** — spin blocking on the ctx semaphore (DISPATCH_TIME_FOREVER via `mov x,0; mvn` in codegen) and re-read `ctx->state` until it equals the cancelled constant. `cancelled` is terminal, so nothing fires afterward. Safe because the handler runs on a *different* dispatch thread, so the blocking wait can't deadlock — provided the exit was NOT already `cancelled` on entry (an accept/close path only enters after its own cancel call, so the cancel always produces the woken transition; connect never cancels before its wait loop).

Load-bearing constants (`src/target/shared/code/tls/macos/`):
- The connection ctx and the listener ctx **share the same prefix** — `CTX_SEM`=0, `CTX_STATE`=16 (mod.rs) — so ONE drain helper (`emit_cancel_drain` in server.rs) serves both, parameterised only by the terminal-state constant.
- Terminal states DIFFER by object: `nw_connection_state_cancelled` = **5**, `nw_listener_state_cancelled` = **4** (listener state_t uses distinct numbering: invalid 0 / waiting 1 / ready 2 / failed 3 / cancelled 4).
- Draining is orthogonal to the leak releases (conn release, queue/sem release): drain waits on the arena ctx semaphore, which `emit_cancel_and_release_conn` does not touch, so release-then-drain or drain-then-release both work. closeListener drains while listener+queue+ctx are still retained (before the releases) as the conservative order.

Verifying without a macOS box: the runtime repro needs concurrent load + process exit and isn't reproducible off-device. Pin it instead with a codegen emit-inspection test (`macos/tests.rs`, `TlsReadTestPlatform`): assert each exit window emits the drain (back-edge label + `ldr_u32 [ctx+CTX_STATE]` + `cmp_imm <cancelled>` + `b.ne` back). The macOS byte-identity golden (`tls_codegen_cover_rt.macos-aarch64.ncodesum`) shifts; the other targets don't (Linux/Windows use their own TLS backends).

### load_selector clobbers caller-saved

`Asm::load_selector` (macos_aarch64/app/mod.rs) emits `local_address(x0,name); call sel_registerName; mov x1,x0` — a real external CALL. It clobbers EVERY caller-saved register (x0-x17), including all `abi::SCRATCH[..]` (x9-x15). So any value computed into scratch and needed AFTER the selector resolves is destroyed.

Symptom: the macOS app term backend built the getCharacters buffer pointer into `SCRATCH[2]`, then called `load_selector`, then used the (clobbered) `SCRATCH[2]` as the `getCharacters:range:` destination → SIGSEGV in `_platform_memmove` (frame: CoreFoundation `_CFStringCheckAndGetCharacters`). Triggered by ANY multi-scalar cluster (NFD combining, ZWJ emoji) that takes the EGC-pool path in `emit_term_draw_text_helper` / `emit_term_write_string_helper`. Fault address varied run-to-run (garbage libobjc-internal value left in the reg), not constant.

Rule: resolve the selector FIRST (it only needs x0/x1), or reload the scratch value AFTER `load_selector`. Do not hold a computed pointer in caller-saved scratch across it. Same family as the `arena_alloc` clobbers x14/x15 bug.

Debugging note: these mfb helpers have a non-standard prologue (no fp frame), so lldb's callee-saved register RECONSTRUCTION for a deep frame is unreliable — it gave plausible-but-wrong `state`/`pool`. Ground truth came from `thread step-inst` through the buffer arithmetic reading the LIVE regs. Also: `.app` binaries load with no ASLR under lldb (`disable-aslr`) but absolute-address breakpoints in the mfb `__text` silently never fire; break on a libobjc symbol (`sel_registerName`) filtered by caller `$lr` instead.

### AudioQueue strands partial buffers

A macOS `AudioQueue` output buffer enqueued with `mAudioDataByteSize` < a full device period is **never completed**: its callback never fires, so it never returns to the free stack. This is not starvation and not a lost wakeup — it is the queue waiting for enough data to fill a period, forever, at end of stream.

This caused `audio::close` to hang ~40-70% of runs on macOS. Fixed by making `write` fill whole buffers only, carrying a short tail in `S_PENDING_BUF`/`S_PENDING_FILL` for the next write, and having `close` pad the leftover with silence before draining.

**Do not enqueue a partial AudioQueue buffer, ever.** Silence-pad instead.

What measurement showed (so it need not be re-derived):
- Exact multiples of `bufferFrames * bytesPerFrame` strand **nothing**, even with a deliberate stall between every write to force the queue dry. Starvation is not the trigger.
- A short tail strands ~50% of runs. Duplicate `AudioQueueStart` after each enqueue (14/25 hangs) and `AudioQueueFlush` before the drain (14/25) both FAIL to fix it — the queue will not finish a short buffer by any means.
- Proof of mechanism: attach to the hung process, `AudioQueueEnqueueBuffer` one more *full* buffer, and the stranded buffer is released and the program exits.

Debugging technique that cracked it: **taking `close` out of the picture** — a probe that writes the PCM then only polls `audio::available` for 2.5 s, never closing, plateaus at 3 of 4 buffers on exactly the runs that would have hung. Do that before suspecting the synchronization. To read the state at a hang: `lldb -p <pid>`, `frame select 2` (the mfb frame), `$st = *(unsigned long*)($sp+0x50)` is the state page (STATE_OFF 48 + a 0x40 saved-register base), then `S_FREE_TOP` at `+0x120`, `S_FREE_BUFS` at `+0x128`, `S_OSOBJECT` at `+0x118`. lldb can call AudioToolbox directly to test remedies live, which beats a rebuild cycle per hypothesis.

## Windows (PE / console / audio)

### Windows codegen verification

`windows-x86_64` codegen **does** have some `.ncodesum` byte-identity goldens and `scripts/artifact-gate.sh` **does** check them — the gate discovers targets from golden *filenames*, so any fixture that ships a `<pkg>.windows-x86_64[.app].ncodesum` golden is cross-compiled and sha256-checked on the macOS host. Confirmed present: `tests/byte-identity/math` (console) and `tests/syntax/app/macos-app-mode-{io,plumbing,term}` (app-mode). So a Windows codegen change to a *covered* fixture IS caught by the gate (it reports `DIFF …windows-x86_64[.app].ncode (sha256)`). Regenerate by building `-ncode -target windows-x86_64 [--app]` and `shasum -a 256 > golden`. `byte-identity/{datetime,http,tls,term,crypto,net,strings,general,io,encoding,regex,audio}` also ship windows `.ncodesum` goldens. (The Windows CNG EC verify fix shifted `byte-identity/crypto`'s windows sum — the other four crypto targets stayed byte-identical, and a base-vs-fix `-ncode` diff confirmed the delta was confined to the six p256/p384/p521 Sign/Verify symbols.)

STALE-GOLDEN TRAP: a change to Windows codegen for a covered fixture that regenerates the OTHER targets' goldens but skips the windows sum leaves the gate RED on `main` for the next person. If you touch Windows codegen, regenerate the windows sum too; if you find one stale, it's a real gate-red to fix (regen blesses the shipped fixed bytes — verify determinism by building twice). Coverage is still partial, so for a Windows path with no golden, verify the two ways below.

MORE INSTANCES: a change to a *shared* Windows emit fans out to EVERY fixture that emits it, so one un-regenerated sum becomes many stale goldens. Example: the FIONBIO ioctl constant `0x8004547E→0x8004667E` left `byte-identity/{http,net,tls}` windows sums stale (every socket program emits the non-blocking ioctl); a Windows app-mode transcript NUL clamp (`win_x86_64/app/mod.rs`) left all three `syntax/app/macos-app-mode-{io,plumbing,term}` `.windows-x86_64.app.ncodesum` stale. When you finalize, run the FULL artifact-gate on `main`, and if a DIFF is NOT your fixture, bisect the cause to the landed commit (`git log <goldenLastRegen>..HEAD -- src/target/win_x86_64 src/target/shared/code/...`), confirm it's an intended/tested change, then regen — do NOT assume it's yours. Watch for a STALE LOCAL BINARY: if `main` advanced under you (concurrent sessions merging), a `-ncode` sha built with an old `target/debug/mfb` gives a false DIFF — `cargo build --bin mfb` at HEAD before trusting a sweep.

1. **CI-observable, host-independent (cross-compile on the macOS host):** `mfb build -target windows-x86_64 -nplan <proj>` dumps the import surface — assert the expected `kernel32`/etc. import with its `requiredBy` symbol (e.g. `SetConsoleOutputCP` `requiredBy "_start"`). `-ncode` dumps ops; the built `.exe` (PE, `MZ` magic) still contains string data as raw bytes (grep the PE for the UTF-8 bytes to prove output isn't transcoded). NOTE: an `entry_imports()` entry in `plan.rs` appears in `-nplan` **even if no code emits the call** — to prove the call is actually emitted, disassemble the PE with `rust-objdump` (`$(rustc --print sysroot)/.../bin/llvm-objdump -d`, ships with the rustup `llvm-tools`; system `objdump`/`llvm-objdump` are absent).

2. **Runtime proof on box 2230** (`ssh -p 2230 test@127.0.0.1`, Win11 x86_64): ssh **pipes** stdout, so the interactive-console *decode* (mojibake) is NOT observable — but console *state* is. For a console-code-page fix, run `chcp 437 & myexe.exe & chcp`: a fresh console defaults to CP 437, and the exe's `SetConsoleOutputCP(65001)` flips it to 65001 (visible in the trailing `chcp`) — a `SetConsole*CP` call persists on the console after the process exits within one `cmd` session. Also assert `echo EXIT=%errorlevel%` == 0 to prove the entry shadow-space frame is sound (the usual Win64 codegen risk).

### Windows codegen emit-inspection test

A Windows/Schannel-only codegen bug can be RED-then-GREEN tested on the macOS host without any Windows box, because the codegen modules are compiled on **all** hosts (`pub(crate) mod schannel;` in `tls/mod.rs` — no `cfg(windows)`). The helpers just append `CodeInstruction`s; they don't execute.

Pattern (`schannel_io.rs`): a `#[cfg(test)] mod` calls the private emit helper directly (the `schannel_*.rs` files are `include!`d into one `schannel` module, so `use super::*` sees every helper) with the shared `crate::target::shared::code::test_support::TestPlatform` — a Linux/AArch64 stub whose `emit_libc_call` just pushes a `bl`, enough to lower + append. **Call the emit helper directly, not a full `lower_*`**, so the returned `Vec<CodeInstruction>` still holds raw `%v8`/`%v9`/offset strings (pre-register-allocation). Then assert on `i.op == CodeOp::StrU64` + `i.get("src"/"base"/"offset")` — e.g. locate the `LdrU64` of the value from `[sp+off]`, grab its `dst`, and assert the next `StrU64` of that reg lands at the right struct offset. This pins the exact ABI fact (pointer must be stored at `SSLPARA+16`, RED at 72 / GREEN at 80) that the bug doc could only state as a "static ABI proof."

Prior art: `openssl.rs` tests use `TestPlatform` + `has_label` the same way. Runs under plain `cargo test --bin mfb`. Complements the whole-pipeline goldens / PE disasm / box 2230 verification — this is the unit-level ABI guard.

Gotcha hit along the way: `cargo fmt --all -- <file>` does NOT scope to that file AND main is not rustfmt-1.9.0-clean, so a tree-wide fmt churns ~90 unrelated files — verify your added block is clean with a scratch-copy `rustfmt --check` instead of running a repo-wide format.

### The compiler's own main-thread stack is 1 MiB on Windows, 8 MiB elsewhere

The front end's depth guards all admit a tree **256 levels deep** (`ast::expr::MAX_EXPR_DEPTH`,
`ast::stmt`'s block cap, `parse_type_name`'s type cap — matched to `ir::verify::check_value_depth`),
and every pass after the parser walks that tree recursively. Those caps were calibrated against
the 8 MiB stack Linux and macOS hand `main`. **Windows reserves 1 MiB**, and 256 levels do not fit
in it: `mfb build` of a 250-group expression died with `0xC00000FD` ("thread 'main' has overflowed
its stack") — both on a hostile shape, *before* its `Expression nesting is too deep.` diagnostic
could print, and on a LEGAL one under the cap. `Test (windows-x86_64)` was the only red row
(bug-542; the guard itself is bug-501).

Fix: `fn main` (`src/main.rs`) spawns the compile on a thread sized by
`COMPILER_STACK_BYTES` (64 MiB — what `ast::expr::tests::on_big_stack` has always used) and exits
101 if it panics, so the compiler runs on a stack *it* chooses rather than whatever the host
reserved for `main`. A thread stack is reserved address space on every supported host, and
`pthread_attr_setstacksize`/Windows' thread reserve are both independent of `RLIMIT_STACK`.

**Rule: a recursion cap is only honest if the compiler HAS the stack that cap costs on the
SMALLEST host stack.** If you raise a depth cap, or add a pass that recurses per tree level, the
budget to check against is `COMPILER_STACK_BYTES`, not the host default.

Repro without a Windows box (Unix only — the PE reserve is a link-time field with no runtime
equivalent to lower): `sh -c 'ulimit -s 1024 && exec mfb build <proj>'`. That is exactly what
`tests/cli_parse_expression_tree_depth.rs`'s two `*_on_a_1mb_main_stack` tests do, so the Windows-
only failure is now reproducible on every Unix row.

### A Win64 emitter must write `return_register()` on EVERY path, not just the error one

Win64's MFB result register is **not** the C result register. plan-85-A aligned
`mfb_return(0)` onto the call-argument bank, so `abi::return_register()` is **`rcx`**
(`CALL_ARGS_WIN64[0]`, `src/arch/x86_64/select.rs`) while `abi::c_return(0)` is **`rax`**.
`emit_linux_c_call` stages `rax → rcx` for every seam routed through it — its comment records
plan-110-D, where the omission had `socket()`/`connect()`/`getsockname()` all checked against
`rcx`, the third *outgoing* argument.

`win_x86_64::call_external` does NOT stage. Any emitter that calls it directly and whose result a
shared caller reads through `return_register()` must stage it itself.

**The trap is a PARTIAL write, not a missing one.** bug-544's `emit_mkstemps` did write
`return_register()` — with `-1`, on the give-up path — and fell through `label(&success)` straight
to `label(&done)` leaving the real handle in `rax`. Grepping for "does this emitter assign the
result register" says yes. The question to ask is **"on every path?"** Its neighbour
`emit_random_bytes` was the plain omission, under a comment asserting the NTSTATUS "is ignored" —
true of the emitter, false of `gen_temp_file`, which sign-extends it and errors on negative.
Between them `fs::createTempFile`, `fs::writeTextAtomic` and `fs::writeBytesAtomic` raised
`7-702-0002 ErrWriteFailed` on Windows unconditionally.

A mechanical sweep needs that per-path question: `emit_path_exists` and `emit_is_terminal` compute
INTO `return_register()` rather than staging `rax`, so grepping for the staging move reports them
as false positives.

**And the coverage lesson, which is why it shipped:** a `#![cfg(unix)]` runtime test plus a
codegen-INSPECTION test look like two tests and are zero Windows runtime coverage.
`rt_fs_create_mode_0600` is unix-only (it asserts `0600`) and `rt_fs_atomic_int_return` only reads
the instruction stream, so the whole atomic-write path was verified on Windows as far as
"it compiles". When a seam is platform-specific, at least one test on it must be
platform-NEUTRAL and must RUN (`tests/rt_fs_temp_and_atomic_write.rs`).

### WASAPI capture carry-over

WASAPI capture requires each `IAudioCaptureClient::GetBuffer` packet be released whole — `ReleaseBuffer(NumFramesRead)` must equal the `GetBuffer` count or 0; you CANNOT partially consume a packet. The `audio::read` loop (`audio/windows_io.rs`, `lower_read`) copies `min(numFrames, framesRemaining)` from the packet then must `ReleaseBuffer(numFrames)` (the whole packet), so on the final packet of any read whose length isn't packet-aligned (the common case) the `numFrames - copyFrames` unconsumed frames are lost → gaps in captured audio. ALSA leaves the remainder in the kernel buffer, macOS in the ring; only Windows dropped it.

Fix = a per-stream carry-over stash in the arena WASAPI STATE block (`windows.rs`): `W_CARRY_PTR` (one device buffer wide = `W_BUFFER * W_MIX_BPF` bytes, allocated at open, INPUT streams only), `W_CARRY_FRAMES` (total stashed), `W_CARRY_HEAD` (frame cursor). The read loop, before `ReleaseBuffer`, copies the unconsumed tail (DEVICE mix format, `W_MIX_BPF` stride) into the stash; the next read DRAINS it first (point `W_OUT1` at `carry+head*mixBpf`, reuse `emit_read_fill`) before touching a new packet. Invariant that keeps it simple: the stash is always empty when a tail is saved, because a read only enters the GetBuffer loop after fully draining the carry (drain resets to 0/0 when `head` reaches `frames`); a partial drain (request < carry) returns before the loop, so no tail-save races a live stash.

Windows-only, untestable at runtime on macOS: proven by emit-inspection tests (`windows_tests.rs`: 2× `emit_read_fill` = drain + per-packet fill; a `*_carry_tail` save label) + a clean `-target windows-x86_64 -ncode` encode. Runtime proof is box 2230. The same fix batch also corrected `audio::available` (returned bytes not frames — dropped the `* BPF`) and rejected `mixCh < userCh` at SHARED open (the read converter reads `userCh` channels/frame, OOB when the device mix has fewer).

### PE trailing sections must chain

In `src/os/windows/link/mod.rs:write_executable`, the PE writer appends optional "trailing" sections after the functional ones (`.text`/`.rdata`/`.data`/`.idata`/`.rsrc`). There are now two: `.mfbnote` (unconditional provenance marker) and `.mfbsign` (signing blob, signed builds only). Each is placed "last, so its size shifts no earlier RVA."

The trap: if each trailing section computes its own slot as `align_up(rsrc_rva + rsrc_bytes.len())` (i.e. "right after `.rsrc`"), then TWO trailing sections both land at the SAME RVA/file offset and **silently overlap** — no error, the section table just has two entries pointing at the same bytes.

Rule: chain them. The unconditional `.mfbnote` is placed after `.rsrc`; `.mfbsign` must be placed after `.mfbnote` (`align_up(mfbnote_rva + mfbnote_bytes.len(), SECTION_ALIGNMENT)`), not after `.rsrc`. Any future third trailing section chains off the last one. Guard added: `signed_build_emits_both_mfbnote_and_mfbsign_disjoint` asserts non-overlapping virtual extents. The write-only linkers have no runtime verifier, so a byte/section scan test is the only guard against this class of bug.

## Staging arguments in place clobbers them when the callee's own arguments share the bank

`canvas::metalDrawScene` stages its arguments into the MFB argument bank, and its own
arguments *arrive* in that bank. Writing `mfb_arg(k)` before reading every `located[j]`
that might live in it is therefore a clobber waiting for the argument count to grow into
it. It did: with a seventh argument, `located[5]` arrived in the register `mfb_arg(5)`
names, so

```
ldr  x5, [x4, #0x8]     ; count  = offsets->count   -> mfb_arg(5)
add  x6, x5, #0x28      ; meta   = located[5] + hdr -> reads the count, not the pointer
add  x7, x6, #0x28      ; cov    = located[6] + hdr -> reads that
```

The failure is a **SIGSEGV at a tiny address** (`0x29` here) on whichever thread runs the
call, with one frame in the crash report and nothing naming the argument. `otool -tV` on
the shipped binary is what localizes it — the three-instruction sequence above is the
whole bug.

**Compute every value into a temporary vreg first, then write the argument registers.**
Reordering the writes happens to work for one mapping and is a trap for the next argument
added. The two-pass form cannot be wrong and the allocator coalesces most of the moves
away. Same family as the scratch-pool/argument-bank aliasing above: a fixed ABI token is
not a variable, and the allocator will not keep a live range in it for you.

## A Windows thread start routine is entered ALREADY 16-byte aligned

`BaseThreadInitThunk` does not leave the 8-byte skew an ordinary `call` does. So the
odd-multiple-of-8 frame that is right for a normal Win64 prologue is wrong for a
`CreateThread` entry, which needs a multiple of 16.

What it costs is not the entry function: the program body's alignment *is* its caller's
call site, so an 8-byte skew there is inherited by **every Win32 call the program ever
makes**. It presents as `0xC0000005` deep inside ntdll — bug-478 faulted in the
activation-context machinery, ~20 frames from anything this repository wrote, on an
empty `SUB main() END SUB`.

And the two entry paths disagree by exactly 8, because the console entry comes from the
PE loader and `entry_stack_misaligned_on_entry` already shaves its skew. So a frame size
measured on one path *breaks* the other. **Test both**, every time — `scripts/test-winapp.sh`
runs the app path and an ordinary `--target windows-x86_64` build runs the console one.

Two guards, both RED-checked: `the_worker_frame_keeps_the_stack_16_byte_aligned`
(`win_x86_64/app/mod.rs`) and `every_calling_win32_seam_reserves_the_callees_shadow_space`
(`win_x86_64/code.rs`). The second covers the other half of that bug: **shadow space is
the caller's job and lands above its own `rsp`**, so an emitter that calls out without
reserving 32 bytes hands the callee 32 bytes of its own locals.
