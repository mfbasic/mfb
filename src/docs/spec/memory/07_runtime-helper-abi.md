# Runtime Helper ABI

A *runtime helper* is a compiler-owned native routine that implements an
OS-touching or otherwise non-inlinable builtin (`audio::`, `io::`, `fs::`,
`net::`, `tls::`, `term::`, `datetime::`, `crypto::`, `os::`, and the `thread::`
family). Source calls to those packages
lower to a `bl` against a stable helper symbol; the helper itself is supplied by
the backend runtime, not by user code, and never appears as a `LINK` import or a
package dependency. This topic owns the *general* helper ABI; the `thread`
family's per-op symbols and direction split are owned by
`./mfb spec threading thread-runtime-helpers`.

Not every builtin becomes a helper call. The `math` and `general` families have
**no** helper specs at all — they are lowered inline. The `strings` family is
likewise emitted inline: every `strings.*` lowering target is intercepted by
`is_native_direct_call` and codegen'd directly, so `strings` has no
`RuntimeHelper` variant and no specs (its dead spec table was removed by
bug-120.1/bug-326). [[src/target/shared/runtime/usage.rs:is_native_direct_call]] [[src/target/shared/runtime/mod.rs:RuntimeHelper]]

## Symbol Scheme

A helper is dispatched at a symbol built by `symbol_for_call(helper, target)`: [[src/target/shared/runtime/mod.rs:symbol_for_call]]

```text
_mfb_rt_<helper>_<call>
```

where `<helper>` is the family name (`audio`, `io`, `fs`, `net`, `tls`, `term`,
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

Each gated helper carries a `RuntimeHelperSpec` — its family, its lowering
target, and a `RuntimeHelperAbi` holding the one machine-read contract fact
that is not derivable from anywhere else: [[src/target/shared/runtime/mod.rs:RuntimeHelperAbi]]

```text
RuntimeHelperSpec
  helper   : RuntimeHelper        ; the family
  call     : &str                 ; the lowering target ("io.print")

RuntimeHelperAbi
  returns  : &str                 ; the result type name ("Nothing" if none)
```

The spec deliberately states nothing that is owned elsewhere (bug-329):

- **The symbol is derived**, never stored: `symbol_for_call(helper, call)`
  produces it, and the catalog tests assert the derivation round-trips for
  every spec. [[src/target/shared/runtime/mod.rs:symbol_for_call]]
- **Argument shapes are owned by the front-end tables** in `src/builtins/`
  (they are what accepts or rejects user code). Arguments are marshalled
  strictly by position into the argument-bank role tokens `%arg0..%arg7`,
  realized per target onto the physical argument registers (AArch64
  `x0..x7`; see `./mfb spec memory native-calling-convention`). There are no
  stack arguments. [[src/target/shared/abi.rs:ARG]]
- **The clobber model is owned by the register allocator**: every internal
  `bl _mfb_*` call destroys the entire caller-saved integer file `x0`–`x17`
  (and `v0`–`v7`), modelled by the allocator's call-clobber masks. Earlier
  revisions carried a per-spec `clobbers` list that understated this set and
  that nothing read; it is gone, and no per-call clobber list exists to be
  trusted.

### Worked example: `io.print`

`io::print(value AS String)` lowers to `_mfb_rt_io_io_print`: [[src/target/shared/runtime/io_specs.rs:IO_PRINT_SPEC]]

```text
symbol   _mfb_rt_io_io_print     ; symbol_for_call(Io, "io.print")
args     value : String in %arg0 ; by position, per the front-end table
returns  Nothing
```

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
Audio  Crypto  Datetime  Fs  General  Io  Math  Net  Os  Term  Thread  Tls
```

Of these, `General` and `Math` are **inline-codegen'd** and contribute no gated
runtime calls (zero specs; the former `Strings` variant was removed outright
when its targets went native-direct). The gated, helper-dispatched families are
`Audio`, `Crypto`, `Datetime`, `Fs`, `Io`, `Net`, `Os`, `Term`, `Tls`, and
`Thread` — the catalog parity test asserts exactly this set is catalogued. [[src/target/shared/runtime/catalog.rs:supported_helper_specs]]
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
actually reached by an emitted call has at least one catalogued spec with a
non-empty return type — rejecting helper families the backend does not
implement. [[src/target/shared/validate.rs:validate_capabilities]] [[src/target.rs:BackendCapabilities]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register result form every helper returns through
* ./mfb spec memory native-calling-convention — argument registers x0..x7 and the infallible-x0 result rule
* ./mfb spec threading thread-runtime-helpers — the `thread` family's per-op symbols and direction split
* ./mfb spec memory arenas — the x19 arena-state register helpers allocate against
* ./mfb spec architecture native — how helper calls are emitted in native codegen
