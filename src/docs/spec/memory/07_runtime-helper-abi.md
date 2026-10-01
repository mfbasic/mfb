# Runtime Helper ABI

A *runtime helper* is a compiler-owned native routine that implements an
OS-touching or otherwise non-inlinable builtin (`audio::`, `io::`, `fs::`,
`net::`, `tcp::`, `udp::`, `tls::`, `term::`, `datetime::`, `crypto::`, `os::`, and
the `thread::` family). Source calls to those packages
lower to a `bl` against a stable helper symbol; the helper itself is supplied by
the backend runtime, not by user code, and never appears as a `LINK` import or a
package dependency. This topic owns the *general* helper ABI; the `thread`
family's per-op symbols and direction split are owned by
`./mfb spec threading thread-runtime-helpers`.

Not every builtin becomes a helper call. The `math` and `general` families have
**no** helper specs at all — they are lowered inline. The `strings` family is
likewise emitted inline: every `strings.*` lowering target is intercepted by
`is_native_direct_call` and codegen'd directly, so `strings` has no
`RuntimeHelper` variant and no specs. [[src/target/shared/runtime/usage.rs:is_native_direct_call]] [[src/target/shared/runtime/mod.rs:RuntimeHelper]]

## Symbol Scheme

A helper is dispatched at a symbol built by `symbol_for_call(helper, target)`: [[src/target/shared/runtime/mod.rs:symbol_for_call]]

```text
_mfb_rt_<helper>_<call>
```

where `<helper>` is the runtime family name and `<call>` is the lowering
target with every non-`[A-Za-z0-9_]` character replaced by `_`. Because the lowering target itself is
already module-qualified (`io.print`, `fs.open`), the `.` is rewritten to `_` and
a target routed through its own package family repeats that package name:

```text
io.print    ->  _mfb_rt_io_io_print
fs.open     ->  _mfb_rt_fs_fs_open
net.lookup  ->  _mfb_rt_net_net_lookup
```

The leading `_mfb_rt_<helper>_` is the runtime-family prefix; the suffix is
the rewritten qualified call. Registry `AbiFunction` members use their
package's family when one exists and `Abi` otherwise; for example, `crypto`
members use `_mfb_rt_abi_crypto_*`. Arena, graph, error, and thread-trampoline
support routines have separate symbol contracts rather than catalog-call names.
[[src/target/shared/runtime/mod.rs:abi_function_family]]

## Per-Helper ABI Descriptor

Each gated helper carries a `RuntimeHelperSpec` — its family, its lowering
target, and a `RuntimeHelperAbi` holding the result spelling and whether it contains an unresolved type
variable: [[src/target/shared/runtime/mod.rs:RuntimeHelperAbi]]

```text
RuntimeHelperSpec
  helper   : RuntimeHelper        ; the family
  call     : &str                 ; the lowering target ("io.print")
  abi      : RuntimeHelperAbi

RuntimeHelperAbi
  returns         : &str          ; the result type spelling ("Nothing" if none)
  returns_generic : bool          ; spelling contains a type variable
```

`returns_generic` prevents the runtime-call resolver from treating an unresolved
result type variable as a concrete nominal type. Catalog specs are frozen from
registry descriptors plus the compiler-only performance helpers.
[[src/target/shared/runtime/catalog.rs:supported_helper_specs]]

Other call properties are derived at their owning layer:

- **The symbol is derived**, never stored: `symbol_for_call(helper, call)`
  produces it, and the catalog tests assert the derivation round-trips for
  every spec. [[src/target/shared/runtime/mod.rs:symbol_for_call]]
- **Argument shapes are owned by the front-end tables** in `src/codegen/builtins/`
  (they are what accepts or rejects user code). Arguments are marshalled
  strictly by position into the MFB argument-convention tokens
  `%argMFB0..%argMFB7` (a typed `Operand::Abi`), realized per target onto the
  physical argument registers (AArch64 `x0..x7`; see `./mfb spec memory
  native-calling-convention`). Additional arguments use the shared outgoing
  stack-tail mechanism.
  [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_prepared_call_args]]
  [[src/target/shared/abi.rs:mfb_arg]]
- **The clobber model is owned by the register allocator.** Internal
  `_mfb_*` calls other than compiled `_mfb_fn_*`/`_mfb_ifn_*` functions
  conservatively clobber every allocatable integer register. FP clobbers
  use the target's caller-saved set. There is no per-spec clobber list.
  [[src/codegen/engine/regalloc/analysis.rs:call_clobber_mask]]

### Worked example: `io.print`

`io::print(value AS String)` lowers to `_mfb_rt_io_io_print`: [[src/codegen/builtins/io/func_print.rs:lower_print]]

```text
symbol   _mfb_rt_io_io_print     ; symbol_for_call(Io, "io.print")
args     value : String in %argMFB0 ; by position, per the front-end table
returns  Nothing
```

## Return Convention

Every gated runtime helper returns through the **four-register fallible result
form** — tag in `%retMFB0`, value in `%retMFB1`, error message in `%retMFB2`, error
source in `%retMFB3` (AArch64 `x0..x3`) — regardless of whether the helper can
actually fail. That ABI (the tags and the four register roles) is owned
by `./mfb spec memory fallible-call-abi`. The ordinary dispatch compares
`%retMFB0` against the OK tag and, on a non-OK tag, stamps the call-site
origin and propagates; on success it reads a non-`Nothing` result from
`%retMFB1`. An inline `TRAP` requests raw dispatch and materializes a
`Result` instead, adopting a parked error where applicable. [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_runtime_helper_call]] [[src/codegen/error/constants/error_constants.rs:RESULT_VALUE_REGISTER]]

A helper that **cannot fail** therefore does not return its value bare in
`%retMFB0` in the way an ordinary infallible callable does (see
`./mfb spec memory native-calling-convention`); instead it sets the OK tag in
`%retMFB0` and places its value in `%retMFB1`. The `datetime` OS-seam helpers
show the form on both sides: `datetime.nowNanos` and `datetime.monotonicNanos`
return an `Integer` with the OK tag set and raise `ErrOverflow` only when a clock
reading's nanosecond count does not fit an `Integer`, and `datetime.localOffset`
raises `ErrInvalidArgument` when `localtime_r` cannot break the instant down into
calendar fields. None of them may be read as a bare `%retMFB0` result
(`./mfb spec stdlib datetime`). [[src/codegen/builtins/datetime/func_now_nanos.rs:lower_now_nanos]] [[src/codegen/builtins/datetime/func_monotonic_nanos.rs:lower_monotonic_nanos]] [[src/codegen/builtins/datetime/func_local_offset.rs:lower_local_offset]] A helper whose `returns` is `Nothing`
yields no value register; only the tag (and, on error, the code/message/source) is
meaningful.

## Helper Families

The helper family is the `RuntimeHelper` enum: [[src/target/shared/runtime/mod.rs:RuntimeHelper]]

```text
Abi App Audio Canvas Datetime Fs General Io Math Net Os Process
Perf Tcp Term Thread Tls Udp
```

`General` and `Math` contribute no gated helper specs; their operations are
lowered inline. Runtime calls in the other source-facing families are derived
from registry descriptors. Packages without a dedicated family, such as
`crypto`, use `Abi`. `helper_for_call` and `supported_helper_specs` share that
routing rule.
[[src/target/shared/runtime/mod.rs:helper_for_call]]
[[src/target/shared/runtime/catalog.rs:supported_helper_specs]]

The `Perf` family is the one **code-layer-only** exception. It has no
MFB language surface and no `helper_for_call` routing — its four helpers
(`perf.init`/`perf.start`/`perf.end`/`perf.done`) are catalogued (so
`spec_for_symbol` resolves them during emission) but injected directly by the code
layer, so they are listed in `CODE_LAYER_ONLY_CALLS` and never take part in the
Required-Helper Analysis or the declared-vs-used parity below. Injection is the
`perf` section of the debug report: it happens only for an **`mfb build --debug`**
build **and** a macOS entry module; a build without `--debug`, and every
Linux/Windows build, emits nothing. The emitted symbols are included in the emitted
runtime-symbol set by the debug registry, independently of source IR.
[[src/target/shared/plan/symbols.rs:runtime_symbols]] See `./mfb spec tooling debug-report`. [[src/codegen/builtins/perf/perf.rs:lower_perf_helper]]

Under the same gate, the program-entry stub opens a whole-program `program` span,
and the arena hot-path helpers `_mfb_arena_alloc` / `_mfb_arena_free` are bracketed
with `perf_start`/`perf_end` (regions `mfb_alloc` / `mfb_free`), so `perf_done`
prints its timing table as the report's `perf` section, from `_mfb_debug_report_perf`
(called by `_mfb_debug_shutdown`, the last call of `_mfb_shutdown`). The perf helpers themselves never touch the
arena (their region is the `emit_arena_map` mmap seam), so bracketing arena code
cannot recurse. [[src/codegen/memory/arena/arena.rs:lower_arena_alloc]]

## Required-Helper Analysis

`required_helpers(ir)` walks the IR and returns the de-duplicated set of helper
families a program actually needs. It scans every value position for a
helper-needing `Call`/`CallResult` (skipping `is_native_direct_call` targets),
and additionally pulls in helpers from two non-obvious sources: [[src/target/shared/runtime/usage.rs:required_helpers]]

- **Resource-union binds.** A bind whose type is a resource union drops by
  dispatching to *each variant's* close op, so binding such a type pulls in
  **every** variant's close helper, not just the active variant's. (A bind of a
  bare resource type pulls in that one type's close helper.) [[src/target/shared/runtime/usage.rs:push_op_helpers]]
- **`.result` member access.** Reading the `result` member of a value pulls in
  the `Thread` family, because thread-result materialization is a thread helper. [[src/target/shared/runtime/usage.rs:push_value_helpers]]

## Validation Invariants

Two backend-shared validators enforce the helper contract on the NIR module.

`validate_nir` recomputes the *used* helper set (mirroring `required_helpers`,
including the resource-union close expansion) and requires the **declared** helper
set to be **exactly equal** to the used set. Both directions are hard errors: [[src/target/shared/validate/mod.rs:validate_nir]]

```text
declared but not used  ->  "NIR declares unused runtime helper '<h>'"
used but not declared  ->  "NIR runtime call requires undeclared helper '<h>'"
```

A helper declared more than once is also rejected.

`validate_capabilities` then gates the module against the backend's advertised
`BackendCapabilities.runtime_calls` set: every emitted (non-native-direct) runtime
call must be a member, or the build fails with "native backend does not support
runtime call '<call>'". It additionally checks that each declared helper that is
actually reached by an emitted call has at least one catalogued spec with a
non-empty return type — rejecting helper families the backend does not
implement. [[src/target/shared/validate/capabilities.rs:validate_capabilities]] [[src/target.rs:BackendCapabilities]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register result form every helper returns through
* ./mfb spec memory native-calling-convention — argument registers x0..x7 and the infallible-x0 result rule
* ./mfb spec threading thread-runtime-helpers — the `thread` family's per-op symbols and direction split
* ./mfb spec memory arenas — the x19 arena-state register helpers allocate against
* ./mfb spec architecture native — how helper calls are emitted in native codegen
