# Runtime Helper ABI

A *runtime helper* is a compiler-owned native routine that implements an
OS-touching or otherwise non-inlinable builtin (`io::`, `fs::`, `net::`, `tls::`,
`term::`, `datetime::`, `crypto::`, `os::`, and the `thread::` family). Source calls to those packages
lower to a `bl` against a stable helper symbol; the helper itself is supplied by
the backend runtime, not by user code, and never appears as a `LINK` import or a
package dependency. This topic owns the *general* helper ABI; the `thread`
family's per-op symbols and direction split are owned by
`./mfb spec threading thread-runtime-helpers`.

Not every builtin becomes a helper call. The `math` and `general` families have
**no** helper specs at all — they are lowered inline. The `strings` family is
likewise emitted inline: every `strings.*` lowering target is intercepted by
`is_native_direct_call` and codegen'd directly, so its specs are never
reached as gated runtime calls. [[src/target/shared/runtime/usage.rs:is_native_direct_call]] [[src/target/shared/runtime/mod.rs:RuntimeHelper]]

## Symbol Scheme

A helper is dispatched at a symbol built by `symbol_for_call(helper, target)`: [[src/target/shared/runtime/mod.rs:symbol_for_call]]

```text
_mfb_rt_<helper>_<call>
```

where `<helper>` is the family name (`io`, `fs`, `net`, `tls`, `term`,
`datetime`, `crypto`, `os`, `thread`) and `<call>` is the lowering target with every non
`[A-Za-z0-9_]` byte replaced by `_`. Because the lowering target itself is
already module-qualified (`io.print`, `fs.open`), the `.` is rewritten to `_` and
the family name appears **twice** — the characteristic doubled-module quirk:

```text
io.print   ->  _mfb_rt_io_io_print
fs.open    ->  _mfb_rt_fs_fs_open
net.close  ->  _mfb_rt_net_net_close
```

The leading `_mfb_rt_<helper>_` is the runtime-module prefix; the trailing
`<helper>_<call>` is the rewritten qualified call. (The `thread` trampoline is
the lone symbol that is not formed this way — see the threading topic.)

## Per-Helper ABI Descriptor

Each gated helper carries a `RuntimeHelperSpec` whose `abi` field is a
`RuntimeHelperAbi` — an explicit, machine-checkable description of the calling
contract: a parameter list, a return type, and a clobber set. [[src/target/shared/runtime/mod.rs:RuntimeHelperAbi]]

```text
RuntimeHelperAbi
  params   : &[RuntimeAbiParam]   ; one entry per argument, in order
  returns  : &str                 ; the result type name ("Nothing" if none)
  clobbers : &[&str]              ; registers destroyed across the call

RuntimeAbiParam
  name     : &str                 ; documentary parameter name
  type_    : &str                 ; the MFBASIC type string
  location : &str                 ; the register the argument arrives in
```

Each `RuntimeAbiParam.location` names the argument-bank role token the argument
is passed in — `%arg0..%arg7`, strictly by position. Each target realizes the
token onto its physical argument register (AArch64 `x0..x7`; the native calling
convention — see `./mfb spec memory native-calling-convention`). There are no
stack arguments. [[src/target/shared/runtime/mod.rs:RuntimeAbiParam]] [[src/target/shared/abi.rs:ARG]]

### Worked example: `io.print`

`io::print(value AS String)` lowers to `_mfb_rt_io_io_print` with this ABI: [[src/target/shared/runtime/io_specs.rs:IO_PRINT_SPEC]]

```text
symbol   _mfb_rt_io_io_print
param    value : String  in %arg0  ; realized to x0 on AArch64
returns  Nothing
clobbers x0, x1, x2, x9, x16
```

The clobber set is the shared constant `IO_PRINT_CLOBBERS`: [[src/target/shared/abi.rs:IO_PRINT_CLOBBERS]]

```text
IO_PRINT_CLOBBERS = [ x0, x1, x2, x9, x16 ]
```

`x0`/`x1`/`x2` are the result-form registers the helper writes (tag, value,
message); `x9` is a runtime scratch register; `x16` is the platform syscall
register. A caller must spill any live value held in those registers across the
`bl`. In practice **every** gated helper declares
`clobbers = IO_PRINT_CLOBBERS`; the field is per-helper so a future helper can
widen its clobber set without changing the dispatch path. [[src/target/shared/runtime/catalog.rs:supported_helper_specs]]

## Return Convention

Every gated runtime helper returns through the **four-register fallible result
form** — tag in `%ret0`, value in `%ret1`, error message in `%ret2`, error
source in `%ret3` (AArch64 `x0..x3`) — regardless of whether the helper can
actually fail. That ABI (the three tags and the four register roles) is owned
by `./mfb spec memory fallible-call-abi`. The dispatch site always compares the
tag in `%ret0` against the OK tag and, on a non-OK tag, stamps the call-site
origin and propagates; on the OK tag it reads the result value from `%ret1`. [[src/target/shared/code/builder_emit_helpers.rs:emit_runtime_helper_call]] [[src/target/shared/code/error_constants.rs:RESULT_VALUE_REGISTER]]

A helper that **cannot fail** therefore does not return its value bare in
`%ret0` in the way an ordinary infallible callable does (see
`./mfb spec memory native-calling-convention`); instead it sets the OK tag in
`%ret0` and places its value in `%ret1`. `datetime.nowNanos` and
`datetime.monotonicNanos` are the canonical infallible-but-result-form helpers:
each returns an `Integer` with the OK tag set. [[src/target/shared/runtime/datetime_specs.rs:DATETIME_NOW_NANOS_SPEC]] `datetime.localOffset` uses the same
result form but *can* fail: it raises `ErrInvalidArgument` (setting the ERR tag)
when `localtime_r` cannot break the instant down into calendar fields, so it must
never be read as a bare `%ret0` result either. A helper whose `returns` is `Nothing`
yields no value register; only the tag (and, on error, the message/source) is
meaningful.

## Helper Families

The helper family is the `RuntimeHelper` enum: [[src/target/shared/runtime/mod.rs:RuntimeHelper]]

```text
Crypto  Datetime  Fs  General  Io  Math  Net  Os  Strings  Term  Thread  Tls
```

Of these, `General`, `Math`, and `Strings` are **inline-codegen'd** and contribute
no gated runtime calls (`Math` and `General` have zero specs; every `Strings`
target is routed through `is_native_direct_call`). The gated, helper-dispatched
families are `Crypto`, `Datetime`, `Fs`, `Io`, `Net`, `Os`, `Term`, `Tls`, and
`Thread`.
`helper_for_call` maps a lowering target to its family. [[src/target/shared/runtime/mod.rs:helper_for_call]]

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
set to be **exactly equal** to the used set. Both directions are hard errors: [[src/target/shared/validate.rs:validate_nir]]

```text
declared but not used  ->  "NIR declares unused runtime helper '<h>'"
used but not declared  ->  "NIR runtime call requires undeclared helper '<h>'"
```

A helper declared more than once is also rejected.

`validate_capabilities` then gates the module against the backend's advertised
`BackendCapabilities.runtime_calls` set: every emitted (non-native-direct) runtime
call must be a member, or the build fails with "native backend does not support
runtime call '<call>'". It additionally checks that each declared helper that is
actually reached by an emitted call has a usable spec — non-empty params, return
type, **and** clobber set — rejecting helpers the backend does not implement. [[src/target/shared/validate.rs:validate_capabilities]] [[src/target.rs:BackendCapabilities]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register result form every helper returns through
* ./mfb spec memory native-calling-convention — argument registers x0..x7 and the infallible-x0 result rule
* ./mfb spec threading thread-runtime-helpers — the `thread` family's per-op symbols and direction split
* ./mfb spec memory arenas — the x19 arena-state register helpers allocate against
* ./mfb spec architecture native — how helper calls are emitted in native codegen
