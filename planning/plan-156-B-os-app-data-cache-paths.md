# plan-156-B: `os::appDataPath` and `os::appCachePath`

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-156-A

This sub-plan adds two calls:

- `os::appDataPath(relative AS String = "") AS String`
- `os::appCachePath(relative AS String = "") AS String`

Each returns the per-user, per-app directory the host platform designates for
application data or cache, joined with `relative`. Both follow the plan-156
family contract (plan-156-A intro): the `.`/`..` component rejection raises
`ErrInvalidPath`, an empty `relative` returns the base with no trailing `/`,
the call creates nothing and checks nothing, and a failed lookup raises
`ErrUnsupported`.

This sub-plan also builds the shared machinery plan-156-C reuses:

- the relative-path validator and the result joiner, factored out of
  `lower_app_resource_path`;
- the POSIX home lookup;
- the Windows known-folder lookup;
- a generic in-place self-update arm for host-path calls.

**Correct behavior.** `<name>` below is the project name.

| Target (console and app builds) | `appDataPath()` | `appCachePath()` |
| --- | --- | --- |
| macOS | `<home>/Library/Application Support/<name>` | `<home>/Library/Caches/<name>` |
| Linux (glibc and musl, AppImage too) | `$XDG_DATA_HOME/<name>` if that variable is set, non-empty and absolute; else `<home>/.local/share/<name>` | `$XDG_CACHE_HOME/<name>` under the same rule; else `<home>/.cache/<name>` |
| Windows | `<FOLDERID_RoamingAppData>/<name>` | `<FOLDERID_LocalAppData>/<name>` |

`<home>` on macOS and Linux is `$HOME` when it is set and non-empty, else
`getpwuid(getuid())->pw_dir`. If neither is available, the call raises
`ErrUnsupported`. Trailing `/` bytes on an environment-derived base are dropped,
except for a lone `/`. On Windows the known-folder path keeps its `\`
separators, and our own joins use `/`, exactly as `appResourcePath` documents
for its base.

References:

- plan-156-A (the family contract, and the prerequisites for all of plan-156).
- `.ai/compiler.md`: the Hard Completion Gate, the fixture rules, and register
  lifetimes across `bl _mfb_*`.
- `.ai/arch-abi.md`: "A platform hook that moves `sp` mid-body". The Win64
  `emit_os_wide_string` window hazard applies to the new known-folder arms.
- `.ai/codegen-invariants.md`: the vreg-allocation order.
- `.ai/testing-gates.md`: private actual dirs, and running the full suite once.
- `.ai/man-content.md`: two new man pages.
- XDG Base Directory Specification 0.8: "All paths set in these environment
  variables must be absolute. If an implementation encounters a relative path
  in any of these variables it should consider the path invalid and ignore it."
- Microsoft `SHGetKnownFolderPath`: the caller must `CoTaskMemFree` `*ppszPath`
  whether the call succeeds or fails.

## Prerequisites

See plan-156-A. On top of that, plan-156-A must be complete:
`rg -c '"os\.resourcePath"' src` → no matches, and every plan-156-A phase has a
filled `Commit:` line.

## 1. Goal

- Both calls resolve, on macOS, Linux and Windows, to the table above.
- A fixture on each OS family proves this at runtime.
- Both calls lower on every native backend. `builtins/tests/os.rs`'s
  every-backend-lowers test is extended to cover them.

### Non-goals (explicit constraints)

- **Nothing is created** and existence is not checked (user decision).
- **No bundle-id directory name.** The directory is `<name>` everywhere (user
  decision), even though Apple's guidance prefers the bundle id.
- **No `appConfigPath`.** Settings go under `appDataPath`. Linux's
  `~/.config` split is deliberately not modeled.
- **No sandbox detection.** A sandboxed macOS app gets whatever `$HOME` says,
  which is its container. That goes in the docs; there is no special code for it.
- **`appResourcePath` codegen does not change** through the §4.1 refactor
  (byte-identical `.ncode`, proven in Phase B1).

## 2. Current State

- **The pieces to factor.** `lower_app_resource_path` (after plan-156-A,
  `src/codegen/builtins/os/func_app_resource_path.rs`) has three parts:
  - It validates `relative` inline: `emit_reject_dot_component` twice, plus a
    scan loop, with `\` also counted on Windows.
  - It joins inline, in its step-4/5 allocate-and-copy block.
  - `gen_shared.rs` already provides `emit_copy_counted`,
    `emit_store_byte_advance`, `alloc_reloc`, `push_alloc_error`,
    `build_string_from_cstr`.
- **POSIX env reading.** `gen_env.rs:lower_get_env` takes the env lock
  (`emit_env_lock`), borrows the name's C string (`borrow_cstring`), calls
  `getenv` (via `platform.emit_external_call`), and releases through
  `emit_env_unlock_return`. That function restores the four result registers
  around the unlock. The lock exists because a concurrent `os::setEnv` can
  relocate `environ` (bug-64), and `OS_ENV_LOCK_CALLS` (`gen_shared.rs`) lists
  every call that takes it. That list gates the lock global's emission
  (`module_uses_env_lock`).
- **The `getpwuid` precedent.** `func_user_name.rs:lower_user_name` takes the
  env lock (it doubles as the pwd lock), calls `getuid` then `getpwuid`, and
  reads `pw_name` at offset 0.
- **The Windows OS-query hook.** `src/target/win_x86_64/code.rs`,
  `emit_os_wide_string(which, …)`:
  - It opens a `subtract_stack(0x60)` window. Slots: `SIZE_SLOT 0x40`,
    `WIDE_SLOT 0x48`, `U8_SLOT 0x50`.
  - It allocates a 4096-byte wide buffer and an 8192-byte UTF-8 buffer.
  - It runs one of `hostName`/`userName`/`executablePath`.
  - It marshals with `emit_wide_slot_to_utf8(from, WIDE_SLOT, U8_SLOT, "8192", …)`.
  - It leaves a NUL-terminated UTF-8 pointer in the return register, or 0 on
    failure.
- **Windows imports.** Per call name, in `src/target/win_x86_64/plan.rs`.
  `SHELL32` and `OLE32` constants already exist (`plan.rs:22`, `:25`), and
  `CoTaskMemFree` is already imported for audio (`plan.rs:736`).
- **The self-update census.** `self_update.rs:self_update_census_covers_every_registry_overload`
  fails for any self-update-shaped member without a `SELF_UPDATE_TABLE` row.
  `Exempt` needs a result "not a function of `x`'s bytes" (`NOT_DERIVED_REASON`),
  which does not fit here, because the result embeds `relative`. `Deferred` is
  a promise, and plan-146-H deleted `Pending` for that reason. So each new
  member needs an **`Arm`**.
- **Hand-maintained call-name tables** that list the sibling `os.appResourcePath`
  (after A). Each new member must be added where its semantics match: see
  plan-156-A §2's census list, plus `OS_ENV_LOCK_CALLS`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Table sites a new `os` path member must join | the 16 files of plan-156-A's census (the tables among them), plus `OS_ENV_LOCK_CALLS` | `rg -n '"os\.appResourcePath"\|"appResourcePath"' src --glob '!src/docs/**'` (re-run after A lands; record the count here) |
| Existing Windows OS-query arms | 3 (`hostName`, `userName`, `executablePath`) | `rg -n '^            "(hostName\|userName\|executablePath)" =>' src/target/win_x86_64/code.rs` |

### Verified properties

- **`struct passwd.pw_dir` offset.**
  - **macOS: 48.** Verified by reading
    `$(xcrun --show-sdk-path)/usr/include/pwd.h`: `pw_name`, `pw_passwd`,
    `uid_t`+`gid_t`, `pw_change`, `pw_class`, `pw_gecos`, then `pw_dir`.
  - **glibc and musl LP64: 32** (`pw_name`, `pw_passwd`, `uid`+`gid`,
    `pw_gecos`, then `pw_dir`). This is **UNVERIFIED against a header here**:
    box 2226 has no `/usr/include/pwd.h`, and 2223/2224 refused ssh on
    2026-09-24. Task B3 proves it at runtime: with `HOME` unset, the result on
    2226 must start with the `pw_dir` that `getent passwd $(id -un) | cut -d: -f6`
    reports.
- **Linux libc reachability.** Linux console builds already import libc
  symbols by name (`readlink`, `getenv`, `getpwuid`), for example
  `gen_paths.rs:emit_executable_path_into` and `func_user_name.rs`. So
  `getpwuid` is reachable on every Linux target, and the "else ErrUnsupported"
  branch fires only when there is no passwd entry.
- **Known-folder GUIDs.** These are UNVERIFIED until the Windows runtime check
  in B4 compares each result against PowerShell `[Environment]::GetFolderPath`.
  A wrong GUID makes `SHGetKnownFolderPath` fail (so the call raises
  `ErrUnsupported`) or return a different folder, and the check catches both.
  - `FOLDERID_RoamingAppData` = `{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}`
  - `FOLDERID_LocalAppData` = `{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}`
- **Does `module_name` equal the manifest `name`?** UNVERIFIED for a call
  compiled inside an imported `.mfp` package. B3's fixture proves it for the
  executable case (the result ends with `/<fixture project name>`). Task B5
  measures the package case.

## 3. Design Overview

Everything is native `Body::abi_function`, like `appResourcePath`.

**Rejected: MFBASIC source bodies (`Body::mfb`).** They would make the Linux
text work easy, but `os` currently injects no source. Adding bodies would inject
`IMPORT fs`/`IMPORT strings` plus code into every program that imports `os`.
The late-pass injection order (`src/ir/lower.rs`, the `http`→`net`/`encoding`/
`color`/`compress` chain) would need a new edge, and the AST/IR goldens of every
`os`-importing fixture would churn. The directory name also has to be a
compile-time constant (`AbiCtx::module_name`), and a static source body cannot
carry it.

The layers, bottom up:

1. **`gen_host_paths.rs` (new).** Shared emitters:
   - `emit_validate_relative` and `emit_join_result`, moved out of
     `lower_app_resource_path` with no change in behavior;
   - `emit_posix_home_base`, the `HOME`→`getpwuid` lookup;
   - `emit_posix_env_abs_base(var)`, an environment variable accepted only when
     it is non-empty and starts with `/`.
2. **`CodegenPlatform::emit_os_wide_string` gains arms `"appData"` and
   `"appCache"`** (Windows only). They call `SHGetKnownFolderPath` and marshal
   UTF-16 to UTF-8 into the existing U8 buffer.
3. **Two member files.** `func_app_data_path.rs` and `func_app_cache_path.rs`
   pick the base per `PlatformFamily`, append the compile-time suffix, and join.
4. **A generic self-update arm, `GrowKind::HostPath`.** It serves both members
   now and C's two later.

**Where the correctness risk sits:**

- **Register lifetimes across external calls.** `getenv`, `getpwuid`, the
  Win64 known-folder window and `_mfb_arena_alloc` each clobber every
  caller-saved register. Every length and pointer carried across them must live
  in a vreg that the allocator spills (the whole `os` body style), or in a frame
  slot. None may sit in a physical register.
- **The Win64 `sub_sp` window.** The known-folder arms name only physical ABI
  registers and their own window slots. That is the invariant
  `gen_paths.rs`'s Windows-arm comment relies on, and
  `tests/codegen/codegen_win64_app_resource_path.rs`-style structural tests pin
  it. B4 adds the same test for the new arms.

**Where the design is uncertain (scheduled first):** B1 does the refactor
with a byte-identity gate, so the shared emitters exist before anything
depends on them.

**Byte-identity.**
- B1 is **provably neutral**. Its gate is byte-identical `.ncode` for the
  `os` byte-identity fixture on all five targets, before and after.
- B2 through B5 add behavior. Their gate is rt-behavior. The `os`
  byte-identity fixture gains the two calls in B5, and all five of its
  `.ncodesum` goldens are expected to change.

## 4. Detailed Design

### 4.1 Shared emitters (`src/codegen/builtins/os/gen_host_paths.rs`)

- **`emit_validate_relative(ctx, arg_data, arg_len, windows, bad_arg, vregs)`.**
  This is `lower_app_resource_path`'s step 1 verbatim: the same labels (derived
  from `ctx.symbol`), the same vreg order, and `\` included on Windows.
- **`emit_join_result(ctx, base: BaseBytes, suffix: &[u8], arg_data, arg_len, alloc_error, done, vregs)`.**
  - It writes `base[..base_len] ++ suffix ++ ("/" ++ relative, only if
    arg_len != 0)` into a fresh arena `String` and sets the OK result.
  - `BaseBytes { ptr, len }` are vregs.
  - `suffix` is compile-time bytes, written with `emit_store_byte_advance`, the
    way `appResourcePath` writes its mode suffix.
  - `appResourcePath` calls it with its `resource_base_offset` suffix as the
    `suffix` bytes, with the separating `/` folded in: `/Resources` for a macOS
    app, `/share/<name>` for a Linux app, and empty for a console or Windows
    build.
  - **B1 must keep `appResourcePath`'s output byte-identical.** If folding the
    leading `/` into the suffix reorders the emitted instructions, keep the old
    order inside `emit_join_result`. The ncode diff decides it.
- **`emit_trim_trailing_slashes(ptr, len)`.** While `len > 1` and
  `ptr[len-1] == '/'`, decrement `len`. This applies only to environment- and
  `pw_dir`-derived bases.
- **`emit_posix_home_base(ctx, fail, vregs) -> BaseBytes`.** Run under the env
  lock, which the caller takes:
  1. Call `getenv("HOME")`, with the name as bytes written into a frame
     scratch area, the way `emit_executable_path_into` writes
     `/proc/self/exe`.
  2. If the result is non-NULL and its first byte is not NUL, use it.
     Otherwise call `getuid`, then `getpwuid`. NULL → `fail`. Otherwise load
     `pw_dir` at **48 on macOS, 32 on Linux**. NULL or empty → `fail`.
  3. Measure the length with a NUL scan (the `strlen` loop pattern already in
     `lower_app_resource_path`), then trim.
- **`emit_posix_env_abs_base(ctx, var, vregs) -> (found_label, BaseBytes)`.**
  Call `getenv(var)`. Use the value only when it is non-NULL, non-empty and
  starts with `/`. Otherwise fall through to the caller's fallback. The result
  is trimmed.

### 4.2 Windows known-folder arms (`src/target/win_x86_64/code.rs`)

`emit_os_wide_string` gains `"appData"` → `FOLDERID_RoamingAppData` and
`"appCache"` → `FOLDERID_LocalAppData`. A private helper
`emit_known_folder(guid: [u8; 16])` inside the arm does this:

1. Write the GUID into window bytes `0x20..0x30` with two `store_u64`
   immediates, in the `Data1` LE / `Data2` LE / `Data3` LE / `Data4` byte-order
   layout. That range is the marshal stack-argument area, which is free before
   the call (only `emit_wide_slot_to_utf8`, which runs after it, uses it). It
   does not overlap `SIZE_SLOT` (0x40), `WIDE_SLOT` (0x48) or `U8_SLOT`
   (0x50), so the window stays 0x60 and no slot moves.
2. Call `SHGetKnownFolderPath(rsp+0x20, 0 /*KF_FLAG_DEFAULT*/, NULL, rsp+WIDE_SLOT)`
   (`SHELL32`). It returns an HRESULT, and non-zero goes to the free-then-fail
   path.
3. This arm skips `arena_alloc_to_slot(…4096…)` for the wide buffer: the
   out-param writes the `CoTaskMem` pointer into `WIDE_SLOT`, which
   `emit_wide_slot_to_utf8` reads. The 8192-byte U8 buffer is still
   arena-allocated.
4. Call `emit_wide_slot_to_utf8`, then `CoTaskMemFree(WIDE_SLOT)` (`OLE32`).
   `CoTaskMemFree` also runs on the failure path; `CoTaskMemFree(NULL)` is a
   no-op.
5. If `WideCharToMultiByte` fails because the 8192-byte buffer is too small,
   the arm must return 0 (fail), **not a truncated path**. Task B4 reads
   `emit_wide_slot_to_utf8` and confirms this. If it does not, the task fixes
   it there, because `hostName` and the others share the same hazard.

`win_x86_64/plan.rs` gains import rows for `os.appDataPath`/`os.appCachePath`:
`SHGetKnownFolderPath` (`SHELL32`), `CoTaskMemFree` (`OLE32`), and
`WideCharToMultiByte` plus the arena symbols the `executablePath` row already
lists.

### 4.3 Member lowerings

**Shared structure.** `lower_app_data_path` / `lower_app_cache_path` share one
body, `lower_app_dir(kind)`:

1. Capture the argument (pointer and length) into vregs **first**. The argument
   registers die at the first external call.
2. Run `emit_validate_relative`.
3. POSIX only: take the env lock.
4. Resolve the base:
   - **macOS:** `emit_posix_home_base`, then suffix
     `"/Library/Application Support/<name>"` or `"/Library/Caches/<name>"`.
   - **Linux:** `emit_posix_env_abs_base("XDG_DATA_HOME"|"XDG_CACHE_HOME")`.
     If found, the suffix is `"/<name>"`. Otherwise use `emit_posix_home_base`
     with suffix `"/.local/share/<name>"` or `"/.cache/<name>"`.
   - **Windows:** `emit_os_wide_string("appData"|"appCache")`. The result is
     NUL-scanned to get its length, and the suffix is `"/<name>"`.
5. Run `emit_join_result`.
6. POSIX: release with `emit_env_unlock_return`, from a single `done` label.
   Windows: `return_()`.

`<name>` is `ctx.module_name`, as compile-time bytes.

**Errors.** `errors: vec!["ErrUnsupported", "ErrInvalidPath"]`. Both are
raised through `raise_error_into`, which the static
`every_raise_error_site_is_declared_in_its_descriptor` scan does not see.
Declare them anyway, as bug-454 did for `appResourcePath`.

### 4.4 In-place self-update arm (`GrowKind::HostPath(&'static str)`)

The census requires this arm (§2). Its semantics equal the copying helper's:
`s = os::appDataPath(s)` produces exactly `os::appDataPath(<old s>)`.

1. Run `emit_reject_dot_components(value_slot)`, reusing the
   `AppResourcePath` arm's validator.
2. Acquire the base by calling the member's own runtime helper with a `""`
   argument:
   - Use `lower_runtime_helper_call`, with the helper from
     `runtime::catalog::spec_for_call(member)`.
   - Materialize the empty `String` through the builder's string-constant
     path; find it with `rg -n 'NirValue::String' src/codegen/engine/value`.
   - `f("")` returns exactly the base. That is the family contract's
     empty-`relative` rule, and the reason A made it hold.
   - The block is statement-scoped. Leave it to the scope drop (do NOT
     `claim_pending_temp` it; nothing caches it).
3. Grow `s` in place by `base_len + (s_len != 0 ? 1 : 0)` and write `base` and
   the conditional `/`. `GrowWrite::Prefix` takes the pointer and length from
   slots. Spill both before any further call.

**Why not cache the base like `AppResourcePath`?** That arm caches because a
process's executable path cannot change. These bases can: `os::setEnv("HOME", …)`
between two statements must change the next result, and the copying helper
honors that. Caching would make the arm disagree with the helper.

**The allocation.** The per-statement base allocation is bounded by the path
length. The arm still avoids what the census exists for: copying `x`.

**Rows.** `STRING_GROW_FNS` gets `("appDataPath", 1..=1, GrowKind::HostPath("os.appDataPath"))`
and the same for `appCachePath`. `STRING_SELF_UPDATE_SPELLINGS` gets both
`os.*` spellings, and `SELF_UPDATE_TABLE` gets rows with `ArmId::StrGrow` and
`str_probe(OS, STR, "os::appDataPath(x)")`.

## Compatibility / Format Impact

- Two new `os` members. No existing contract changes.
- New Windows imports (`SHGetKnownFolderPath` from `shell32.dll`; `ole32.dll`
  is already a dependency when audio is used), only in programs that call these
  members. Import rows are per call.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick in the same commit as
> the work. Use `- [~]` for partial, mark moot as `- [x] ~~text~~ — moot:
> <evidence>`, and fill `Commit:` on landing. **An unticked box means NOT DONE.**

### Phase B1: factor the shared emitters (provably neutral)

- [ ] Create `src/codegen/builtins/os/gen_host_paths.rs` with
      `emit_validate_relative`, `emit_join_result` and
      `emit_trim_trailing_slashes` (§4.1), and register it in `os/mod.rs`.
- [ ] Rewrite `lower_app_resource_path` onto `emit_validate_relative` and
      `emit_join_result`.
- [ ] Before the change, build the `os` byte-identity fixture with `-ncode` for
      the five targets (`macos-aarch64`, `linux-x86_64`, `linux-aarch64`,
      `linux-riscv64`, `windows-x86_64`) into `/tmp/156b1-before/`, and after
      the change into `/tmp/156b1-after/`.

Acceptance: `appResourcePath` emits the same bytes as before.
  Check: `for t in …; do cmp /tmp/156b1-before/$t.ncode /tmp/156b1-after/$t.ncode; done`
  → no output (est. 3 min). A diff means the refactor changed codegen. Objdump
  that ONE target, fix the emitter, and re-run. It is not a reason to abandon
  the factoring.
Commit: —

### Phase B2: POSIX lookups and the macOS/Linux lowering

- [ ] Add `emit_posix_home_base` and `emit_posix_env_abs_base` to
      `gen_host_paths.rs` (§4.1).
- [ ] Add `func_app_data_path.rs` and `func_app_cache_path.rs` with their
      descriptors (`relative` defaulting to `""`, both errors) and the
      `lower_app_dir(kind)` body for macOS and Linux (§4.3). Write the man
      prose per `.ai/man-content.md`:
  - what each directory is for (data versus regenerable cache);
  - the per-OS table;
  - "does not create the directory — create it with `fs::createDirectories`
    before writing";
  - the sandboxed-macOS container sentence;
  - an example that creates the directory, then writes a file.
- [ ] Add `"os.appDataPath"`, `"os.appCachePath"` to `OS_ENV_LOCK_CALLS`.
- [ ] Add both calls to every matching table from plan-156-A's census:
  - `data_objects.rs`: `ErrUnsupported` and `ErrInvalidPath`;
  - the `linux_common`/`macos_aarch64` supported-call lists, and their
    `plan.rs` import rows (`getenv`, `getuid`, `getpwuid`,
    `pthread_mutex_lock`/`unlock`, arena);
  - `registry/mod.rs`'s two `String`-result lists;
  - `audit/collect/source.rs`: capability `"environment"` (it reads the
    environment), and the fallible-call list.
- [ ] Windows lowering is B4. Until B4 lands, **do not** add the calls to
      `win_x86_64/mod.rs`'s supported list. B2 and B4 must land in the same
      push (see the B4 note).

Acceptance: on macOS both calls return the table's values under controlled
environments.
  Check: `target/debug/mfb build tests/rt-behavior/os/func_os_appDataPath_valid && …/build/*.out`
  prints the fixture's expected lines (written in B3) (est. 2 min).
Commit: —

### Phase B3: fixtures and the POSIX runtime proof

- [ ] Add `tests/rt-behavior/os/func_os_appDataPath_valid` and
      `func_os_appCachePath_valid`. Each program sets its own environment with
      `os::setEnv`/`os::unsetEnv`, branches on `os::name()`, and prints only
      `TRUE`/`FALSE`/error-code lines. That makes the golden identical on
      every OS. Cases:
  - **`HOME=/tmp/mfb156`:** `appDataPath()` equals the OS row with that home
    (macOS: `/tmp/mfb156/Library/Application Support/<fixture name>`; Linux:
    with `XDG_DATA_HOME` unset, `/tmp/mfb156/.local/share/<fixture name>`;
    Windows: equals `os::getEnv("APPDATA") & "/" & name`).
  - **`HOME=/tmp/mfb156/`:** the trailing slash is trimmed, so the same result.
  - **Linux `XDG_DATA_HOME=/tmp/x`:** `/tmp/x/<name>`. **`XDG_DATA_HOME=rel`:**
    ignored, so the `HOME` result. **`XDG_DATA_HOME=""`:** ignored.
  - **`appDataPath("saves/a.dat")`** equals `appDataPath() & "/saves/a.dat"`.
  - **`appDataPath()`** does not end with `/`.
  - **`appDataPath("a/../b")`, `("../x")`, `("x/..")`, `("./x")`** each raise
    `77030002`. On Windows, `("..\x")` also raises `77030002`.
  - **`HOME` unset (POSIX):** the result starts with `/` and ends with the
    per-OS suffix (the `getpwuid` fallback).
  - **In place:** `MUT s = "saves/a.dat"` / `s = os::appDataPath(s)` equals the
    copying result, and `s = ""` in place equals `appDataPath()`.
  - **In place after `os::setEnv("HOME", "/tmp/other")`:** the next in-place
    result uses the new home (proves the arm does not cache).
  - The same set for `appCachePath`, with `XDG_CACHE_HOME`.
- [ ] Add `tests/syntax/os/func_os_appDataPath_invalid` and
      `func_os_appCachePath_invalid`, covering a wrong type and a wrong arity.
- [ ] Linux runtime:
      `FILTER=func_os_app scripts/linux-runtime-proof.sh target/debug/mfb 2226 linux-aarch64 glibc`
      → both fixtures match. Add one `/tmp` probe run on 2226 that compares
      the `HOME`-unset result against `getent passwd $(id -un) | cut -d: -f6`.
      This settles the Linux `pw_dir` offset of 32 (§2).

Acceptance: both fixtures pass locally and on 2226, and the `getent` comparison
matches.
  Check: `FILTER=func_os_app scripts/test-accept.sh target/debug/mfb target/accept-actual-156b`
  → 4 new fixtures written and nothing else changed (est. 3 min). Then run the
  2226 command above → all ok (est. 5 min; box 2226 because it is the only
  reachable glibc Linux box, per the plan-156-A prerequisites).
Commit: —

### Phase B4: Windows known-folder lowering

> B2's POSIX commit must not be pushed on its own if Windows is missing,
> because the calls would not lower on `windows-x86_64`. Land B2 through B4 as
> consecutive commits in one push.

- [ ] Add the `"appData"`/`"appCache"` arms and `emit_known_folder` in
      `src/target/win_x86_64/code.rs` (§4.2). Put the GUID in window bytes
      `0x20..0x30`.
- [ ] Read `emit_wide_slot_to_utf8`. It must return failure, not a truncated
      string, when the 8192-byte buffer is too small. Record the evidence here;
      if it truncates, fix it (this affects `hostName`/`userName`/
      `executablePath` too).
- [ ] Add the Windows arm of `lower_app_dir`, the `win_x86_64/plan.rs` import
      rows, and the `win_x86_64/mod.rs` supported-call entries.
- [ ] Add a structural test in `tests/codegen/`, mirroring
      `codegen_win64_app_resource_path.rs`'s
      `nothing_addresses_outside_the_windows_acquisition_frame`, for
      `os.appDataPath` and `os.appCachePath`. Register it in `Cargo.toml`.
- [ ] Extend `builtins/tests/os.rs`'s every-backend-lowers test to cover both
      calls.
- [ ] Windows runtime proof on 2230 (a one-off in `/tmp`, not `scripts/`):
  1. Build both B3 fixtures with `-target windows-x86_64`.
  2. Ship them with `win_ship` (`scripts/remote-common.sh`).
  3. Run them, and diff the output against the golden `build.log` run tail.
  4. Separately, print `os::appDataPath()` and compare it with
     `powershell -c "[Environment]::GetFolderPath('ApplicationData')"` +
     `/<name>`; for `appCachePath` use `'LocalApplicationData'`.

Acceptance: both fixtures match their goldens on 2230, and both results match
PowerShell's folders.
  Check: the 2230 run above → identical output, and the two equality lines are
  `TRUE` (est. 8 min: the build is local, and ship+run is about 1 min per
  fixture; nothing smaller exercises `SHGetKnownFolderPath`).
Commit: —

### Phase B5: byte-identity coverage and the package-name probe

- [ ] Add `io::print(os::appDataPath("d"))` and
      `io::print(os::appCachePath("c"))` to
      `tests/byte-identity/os/src/main.mfb`. The five `.ncodesum` goldens are
      regenerated in plan-156-D, after the full suite.
- [ ] Probe (in `/tmp`): a `.mfp` package whose function returns
      `os::appDataPath()`, called from an executable project named `host`.
      Record whether the result ends with `/host` or with the package's name.
      If it ends with the package's name, that is a bug in `module_name`
      threading. Fix it here: the directory must be the executable's name,
      because that is the app.

Acceptance: the probe result is recorded in Corrections, and it ends with
`/host` (after a fix if one was needed).
  Check: the probe's stdout (est. 3 min).
Commit: —

## Validation Plan

- **Tests:** two valid rt-behavior fixtures and two invalid syntax fixtures;
  the extended every-backend-lowers unit test; the Win64 window structural
  test; and the self-update census, which passes with the two new rows.
- **Coverage check:** the in-place lines in the valid fixtures must take the
  `HostPath` arm. Confirm with `--nir` and an ncode inspection on one fixture:
  at the self-update site, the helper is called with the empty constant, not
  with `s`.
- **Runtime proof:** macOS (local), Linux aarch64 glibc (2226), Windows (2230).
  - Linux x86_64, musl and riscv64 are **not executed**, because their boxes
    refused ssh on 2026-09-24. Their codegen is covered by the byte-identity
    `.ncodesum` goldens and the lowering tests. plan-156-D re-probes those
    boxes and runs them if any answers.
- **Doc sync:** the two man pages (descriptor prose) and
  `src/docs/spec/stdlib/14_os.md`. The spec gets a new "Per-user app
  directories" section with the table, the XDG absolute-path rule, the
  `HOME`→`getpwuid` fallback, the known-folder mapping and `[[path:Symbol]]`
  citations, and the two error-table rows gain both calls. Gate:
  `cargo test --bin mfb spec`, `scripts/spec-census.sh --citations`.
- **Final gate:** plan-156-D.

## Open Decisions

- None open. The trailing-slash trim of environment-derived bases is a design
  default this plan sets, following the family contract ("no trailing `/`").
  A lone `/` home stays `/`, so `appDataPath()` becomes
  `/Library/Application Support/<name>`, not `//Library…`.

## Corrections

## Summary

The engineering risk is in two places:

- register lifetimes across four kinds of external call inside one POSIX body;
- the Win64 `sub_sp` window for the new known-folder arms.

The refactor comes first, behind a byte-identical gate. The self-update arm
deliberately re-acquires the base on each statement, so it matches the copying
helper under `setEnv`.
