**Repo State**
Clean worktree.

Latest commits:
```text
66c78a5 Split native target and AArch64 backend policy
df0d3c1 Add linux aarch64 native target
577d7b9 Add macOS aarch64 native backend pipeline
```

**Current Native Pipeline**
Compiler native flow is now:

```text
Bytecode/IR
  -> target/shared/lower.rs       (-nir)
  -> target/*_aarch64/plan.rs     (-nplan)
  -> os/{macos,linux}/object.rs   (-nobj)
  -> target/*_aarch64/code.rs     (-ncode)
  -> arch/aarch64/encode.rs
  -> os/{macos,linux}/link.rs
  -> executable
```

`-bin` was removed earlier. Native inspection outputs are now:
- `-nir`
- `-nplan`
- `-nobj`
- `-ncode`

**Ownership Split**
Target selection lives in [src/target.rs](/Users/justinzaun/Development/mfb/src/target.rs:1).

Backends:
- [src/target/macos_aarch64](/Users/justinzaun/Development/mfb/src/target/macos_aarch64/mod.rs:1)
- [src/target/linux_aarch64](/Users/justinzaun/Development/mfb/src/target/linux_aarch64/mod.rs:1)
- [src/target/package_mfp](/Users/justinzaun/Development/mfb/src/target/package_mfp/mod.rs:1)

Shared native pipeline:
- [src/target/shared/lower.rs](/Users/justinzaun/Development/mfb/src/target/shared/lower.rs:1): bytecode/IR to NIR
- [src/target/shared/nir.rs](/Users/justinzaun/Development/mfb/src/target/shared/nir.rs:1): NIR data model and JSON
- [src/target/shared/plan.rs](/Users/justinzaun/Development/mfb/src/target/shared/plan.rs:1): generic native plan lowering, with target import policy injected
- [src/target/shared/code.rs](/Users/justinzaun/Development/mfb/src/target/shared/code.rs:1): shared native code-plan lowering
- [src/target/shared/runtime.rs](/Users/justinzaun/Development/mfb/src/target/shared/runtime.rs:1): runtime helper identity/ABI metadata
- [src/target/shared/validate.rs](/Users/justinzaun/Development/mfb/src/target/shared/validate.rs:1): shared validation

Architecture:
- [src/arch/aarch64/abi.rs](/Users/justinzaun/Development/mfb/src/arch/aarch64/abi.rs:1): AArch64 registers, ABI helpers, instruction builders
- [src/arch/aarch64/ops.rs](/Users/justinzaun/Development/mfb/src/arch/aarch64/ops.rs:1): AArch64 code-plan op vocabulary
- [src/arch/aarch64/encode.rs](/Users/justinzaun/Development/mfb/src/arch/aarch64/encode.rs:1): code plan to bytes

OS/container:
- [src/os/macos/object.rs](/Users/justinzaun/Development/mfb/src/os/macos/object.rs:1): Mach-O object plan
- [src/os/macos/link.rs](/Users/justinzaun/Development/mfb/src/os/macos/link.rs:1): Mach-O executable/linking
- [src/os/linux/object.rs](/Users/justinzaun/Development/mfb/src/os/linux/object.rs:1): ELF object plan
- [src/os/linux/link.rs](/Users/justinzaun/Development/mfb/src/os/linux/link.rs:1): ELF executable/linking

**Platform Policy**
macOS AArch64:
- [src/target/macos_aarch64/plan.rs](/Users/justinzaun/Development/mfb/src/target/macos_aarch64/plan.rs:1) adds `libSystem` imports: `_exit`, `_write`.
- [src/target/macos_aarch64/code.rs](/Users/justinzaun/Development/mfb/src/target/macos_aarch64/code.rs:1) emits calls to `_exit` and `_write`.

Linux AArch64:
- [src/target/linux_aarch64/plan.rs](/Users/justinzaun/Development/mfb/src/target/linux_aarch64/plan.rs:1) has no platform imports.
- [src/target/linux_aarch64/code.rs](/Users/justinzaun/Development/mfb/src/target/linux_aarch64/code.rs:1) emits direct Linux syscalls: `write = 64`, `exit = 93`.

**Runtime Helpers**
Currently implemented native runtime helper:
- `io.print`
- Emits the string contents and then a newline.
- macOS implementation calls `_write`.
- Linux implementation uses direct syscall write.

`_mfb_rt_io_io_print` is internal runtime helper code in `-ncode`; `_write` is the macOS platform import.

**Supported Native Behavior**
Native path currently supports the tested subset:
- project entry selection
- non-`main` entry cases covered by project-entry tests
- entry function/sub signature variants covered by tests
- `io.print`
- integer constants and addition
- booleans/control-flow conditions used by tests
- `if`
- `match` over currently tested enum/union-style patterns
- string literals as data objects
- simple local binding/assignment/return/call paths used by acceptance tests

Unsupported/native-incomplete areas still exist:
- general heap allocation
- real strings beyond current literal layout/use
- file ops
- memory allocation
- thread runtime
- full function pointer/call support
- stack-passed arguments
- list/map literals
- many built-ins outside `io.print`
- broader ABI work for complex types/returns

**Validation Last Run**
Before commit `66c78a5`, these passed:
```text
cargo fmt
cargo test
cmake --build build
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

Also ran local macOS native smoke:
```text
target/debug/mfb build tests/parser-hello-world
tests/parser-hello-world/parser_hello_world.out
```

Output:
```text
Hello World
```

Linux AArch64 was previously runtime-tested on the VM after `df0d3c1`, but after the latest split I did not complete VM runtime execution because SSH on `127.0.0.1:2222` was refusing connections during the previous attempt. Acceptance and local build paths still pass.

**Good Next Steps**
1. Rename `target/shared` if desired. It is now mostly shared pipeline, but it still imports `arch::aarch64` for the current native code plan. A future split could become:
   - `target/shared`: truly target-neutral NIR/plan/runtime validation
   - `target/aarch64_shared`: AArch64 native code-plan lowering
2. Add explicit target-level validation so `macos_aarch64` rejects non-macOS and `linux_aarch64` rejects non-Linux before shared validation.
3. Re-run Linux VM runtime smoke once SSH is available.
4. Expand runtime helpers one at a time, starting with the next built-in needed by tests.
