# Native IR (NIR)

The Native IR is the last shared, target-independent representation before
per-target code generation. `lower_module` lowers an `IrProject` into a
`NirModule`; the structure mirrors the shared IR almost one-for-one, with three
classes of change layered on top: builtin calls are split into **native-direct**
vs **runtime-call** routing, native `LINK` functions are turned into
call-routing imports plus a load-time initializer, and every emitted entity is
assigned its final, mangled link symbol. The shared IR it lowers from is
`./mfb spec architecture ir`; the concrete `_mfb_fn_` callee names it consumes
originate in `./mfb spec architecture monomorphization`.
[[src/target/shared/nir/mod.rs:NirModule]] [[src/target/shared/nir/lower.rs:lower_module]]

By the time `lower_module` runs, imported packages have already been decoded and
merged into the `IrProject` (see `merge_packages`), so every function flows
through this one lowering and there are no package-level imports left to resolve.
[[src/target/shared/nir/lower.rs:merge_packages]]

## Module shape

```text
NirModule {
  target          : String                  // e.g. "macos-aarch64"
  build_mode       : NativeBuildMode          // console | macos-app
  project          : String
  entry            : Option<NirEntryPoint>
  globals          : Vec<NirGlobal>
  types            : Vec<NirType>            // type | union | enum
  imports          : Vec<NirImport>         // LINK call-routing entries
  runtime_helpers  : Vec<RuntimeHelper>     // helper groups this module needs
  functions        : Vec<NirFunction>
  link_functions   : Vec<IrLinkFunction>    // carried verbatim from IR
}
```

If the project has any global bindings, `lower_functions` prepends a synthetic
private SUB named `__mfb_init_globals_<project>` whose body is one `StoreGlobal`
per binding; it runs before user code to materialize global initializers.
[[src/target/shared/nir/lower.rs:lower_functions]]
[[src/target/shared/nir/lower.rs:lower_global_initializer]]
[[src/target/shared/nir/symbols.rs:global_initializer_name]]

## Op set

`NirOp` is the statement vocabulary inside a function body. Lowering from `IrOp`
is structural (`lower_op` recurses through nested bodies via `lower_ops`); the
only renames are `IrOp::AssignGlobal` -> `StoreGlobal` (with an empty `type_`
and a wrapped value) and otherwise a 1:1 mapping.
[[src/target/shared/nir/lower.rs:lower_op]] [[src/target/shared/nir/mod.rs:NirOp]]

| Op | JSON `"op"` | Fields | Notes |
|----|-------------|--------|-------|
| `Bind` | `bind` | `mutable`, `name`, `type`, `value?` | `LET`/`MUT` local binding |
| `StoreGlobal` | `storeGlobal` | `name`, `type`, `value?` | global init + `IrOp::AssignGlobal` (empty `type`) |
| `Assign` | `assign` | `name`, `value` | reassign a `MUT` local |
| `StateAssign` | `stateAssign` | `resource`, `value` | replace a `RES` binding's `STATE` payload |
| `Return` | `return` | `value?` | |
| `ExitLoop` | `exitLoop` | `loop` (`for`/`do`/`while`) | |
| `ContinueLoop` | `continueLoop` | `loop` | |
| `ExitProgram` | `exitProgram` | `code` | |
| `Fail` | `fail` | `error` | raise into the enclosing `TRAP`/result |
| `Eval` | `eval` | `value` | evaluate for effect, discard result |
| `If` | `if` | `condition`, `then[]`, `else[]` | |
| `Match` | `match` | `value`, `cases[]` | cases carry pattern + optional guard |
| `While` | `while` | `loop`, `condition`, `body[]` | pre-test loop |
| `For` | `for` | `name`, `type`, `start`, `end`, `step`, `body[]` | header `loc` drives increment-overflow origin (not serialized) |
| `DoUntil` | `doUntil` | `condition`, `body[]` | post-test loop |
| `ForEach` | `forEach` | `name`, `type`, `iterable`, `body[]` | |
| `Trap` | `trap` | `name`, `body[]` | error-handler region |

`LoopKind` serializes as the lowercase strings `for` / `do` / `while`.
[[src/target/shared/nir/json.rs:loop_kind_name]]

A `Match` case is a `NirMatchPattern` (`Else`, `Value(v)`, or `OneOf([v…])`),
an optional guard value, and a body. Patterns serialize with `"kind"` of
`else` / `value` / `oneOf`. [[src/target/shared/nir/mod.rs:NirMatchPattern]]

## Value taxonomy

`NirValue` is the expression vocabulary. Most variants are a structural copy of
the matching `IrValue`. Two variants carry no IR counterpart and are *introduced*
during NIR lowering: `RuntimeCall` (a builtin routed to a runtime helper, see
below) and `Global` gains a `type_` field that lowering fills with an empty
string (the IR `Global` is just a name). [[src/target/shared/nir/lower.rs:lower_value]]
[[src/target/shared/nir/mod.rs:NirValue]]

| Value | JSON `"kind"` | Shape |
|-------|---------------|-------|
| `Const` | `const` | `type`, `value` (both strings) |
| `Local` | `local` | `name` |
| `LocalRef` | `localRef` | `name`, `type` — address of a slot (a reference), to capture a `MUT` into a non-escaping callback env |
| `Global` | `global` | `name`, `type` (`type` empty after lowering) |
| `FunctionRef` | `functionRef` | `name`, `type` |
| `Closure` | `closure` | `name`, `type`, `captures[]` |
| `Capture` | `capture` | `index`, `type`, `byRef` (emitted only when true) |
| `Call` | `call` | `target`, `args[]` — user fn or native-direct builtin |
| `CallResult` | `callResult` | `target`, `args[]` — fallible call returning a Result |
| `RuntimeCall` | `runtimeCall` | `helper`, `target`, `args[]` — **NIR-only**, builtin routed to a runtime helper |
| `Constructor` | `constructor` | `type`, `args[]` |
| `UnionWrap` | `unionWrap` | `union`, `member`, `value` |
| `UnionExtract` | `unionExtract` | `type`, `value` |
| `ResultIsOk` | `resultIsOk` | `value` |
| `ResultValue` | `resultValue` | `value` |
| `ResultError` | `resultError` | `value` |
| `WithUpdate` | `with` | `type`, `target`, `updates[]` (`field`/`value`) |
| `ListLiteral` | `list` | `type`, `values[]` |
| `MapLiteral` | `map` | `type`, `entries[]` (`key`/`value`) |
| `MemberAccess` | `memberAccess` | `target`, `member` |
| `Binary` | `binary` | `op`, `left`, `right` |
| `Unary` | `unary` | `op`, `operand` |

`Call`, `CallResult`, `RuntimeCall`, `Binary`, and `Unary` each carry a
`NirSourceLoc { line, column }` for runtime-error attribution; the file is on
the owning `NirFunction::file`. The `loc` is *not* serialized to JSON.
[[src/target/shared/nir/mod.rs:NirSourceLoc]]

### Traversal seam

Analyses that recurse over `NirOp`/`NirValue` share one traversal: the
`NirVisitor` trait and its `walk_ops`/`walk_op`/`walk_value` free functions in
[[src/target/shared/nir/visit.rs]]. An analysis implements the trait, overrides
only the nodes it cares about, and inherits complete recursion for the rest;
adding a `NirOp`/`NirValue` variant is a compile error in the one `walk_*`
function rather than a silent gap across the many collectors. The `walk_op`
recursion for `Match` visits the scrutinee, the pattern's values, the
`WHEN … WHERE` **guard**, and the body — walking the guard is a load-bearing
invariant (a runtime call or platform import used only in a guard still executes;
see bug-118, bug-328). The IR value tree has the analogous depth-bounded
`visit_value`/`visit_value_mut` seam beside `IrValue` in
[[src/ir/value.rs]]. Scope-sensitive analyses that thread a per-branch constants
map, and the code-emitting lowering passes, keep that state themselves and are
written directly against the enums rather than the shared seam.

## Call routing: native-direct vs runtime-call

The one semantically interesting rewrite happens in `lower_value` for
`IrValue::Call`. After the args are lowered, the target name decides the routing:

1. `is_native_direct_call(target)` -> keep as `NirValue::Call`. These builtins
   are lowered inline by the backend (no helper). The set includes every
   `native_builtin_target`, plus a fixed list: `len`, the `fs.path*` family,
   the numeric coercions (`toByte`/`toFixed`/`toFloat`/`toInt`/`toString`), the
   `is*` predicates, and the `math.abs`/`math.min`/… inline-math group.
   [[src/target/shared/runtime/usage.rs:is_native_direct_call]]
2. Otherwise `helper_for_call(target)` -> if it returns a `RuntimeHelper`,
   rewrite to `NirValue::RuntimeCall { helper, target, args, loc }`. The helper
   is one of `Crypto`, `Datetime`, `Fs`, `General`, `Io`, `Math`, `Net`, `Os`,
   `Strings`, `Term`, `Thread`, `Tls`; the symbol the backend calls is
   `_mfb_rt_<helper>_<sanitized-target>` (`./mfb spec memory runtime-helper-abi`).
   [[src/target/shared/runtime/mod.rs:helper_for_call]]
   [[src/target/shared/runtime/mod.rs:RuntimeHelper]]
3. Otherwise (a user function) -> stay `NirValue::Call`; the backend mangles
   `target` to `_mfb_fn_…` at code-gen time.

`CallResult` is never re-routed here — it is copied straight through as a
fallible call (`./mfb spec memory fallible-call-abi`).

### Builtin default-argument rewrites

Two builtins get a synthetic trailing argument synthesized during lowering, so
the backend always sees a fixed arity: [[src/target/shared/nir/lower.rs:lower_value]]

```text
fs.openFile(path)          -> append Const String "read"         (mode default)
fs.openFileNoFollow(path)  -> append Const String "read"
fs.createTempFile()        -> append RuntimeCall fs.tempDirectory  (dir default)
```

The injected `fs.tempDirectory` is itself a `RuntimeCall` (helper `Fs`), so the
0-arg temp-file form resolves the system temp directory at runtime.

## Symbol mangling

NIR is where final link symbols are assigned. All fragments are sanitized by
`symbol_fragment`: every character outside `[A-Za-z0-9_]` becomes `_`.
[[src/target/shared/nir/symbols.rs:symbol_fragment]]

| Entity | Helper | Form |
|--------|--------|------|
| User function | `function_symbol` | `_mfb_fn_<fragment(name)>` |
| Internal (sigil `#…`) function | `function_symbol` | `_mfb_ifn_<fragment(rest)>` |
| Global | `global_symbol` | `_mfb_global_<fragment(project)>_<fragment(name)>` |
| Global initializer | `global_initializer_name` | `__mfb_init_globals_<fragment(project)>` |
| Runtime helper call | (backend) | `_mfb_rt_<helper>_<sanitized(target)>` |
| `LINK` thunk | `link_thunk_symbol` | `_mfb_linker_<sanitize(alias)>_<sanitize(name)>` |
| `LINK` init | `LINK_INIT_SYMBOL` | `_mfb_linker_init` |

`function_symbol` keys off `internal_name::strip_sigil`: a sigil-prefixed
(`#`) name lands in the reserved `_mfb_ifn_` namespace so a compiler-injected
builtin can never collide with a user function (always `_mfb_fn_`).
[[src/target/shared/nir/symbols.rs:function_symbol]]
[[src/target/shared/nir/symbols.rs:global_symbol]]
[[src/internal_name.rs:strip_sigil]]
How these symbols are emitted and relocated is `./mfb spec linker
symbols-and-relocations`.

## Native LINK routing

`link_routing_imports` walks `ir.link_functions` and, for each, emits a
`NirImport` whose `name` is `alias.func` and whose `symbol` is its
`link_thunk_symbol`. This re-uses the ordinary import-resolution path so a call
to a native function dispatches to its generated marshaling thunk. A re-export
alias (`ir.link_aliases`) gets a second import routing to the same thunk, under
the bare alias name. [[src/target/shared/nir/lower.rs:link_routing_imports]]
[[src/target/shared/nir/mod.rs:link_thunk_symbol]]

`LINK_INIT_SYMBOL` (`_mfb_linker_init`) names the per-program load-time
initializer that runs `dlopen`/`dlsym` before `main`.
[[src/target/shared/nir/mod.rs:LINK_INIT_SYMBOL]] See `./mfb spec linker
package-linking` and `./mfb spec language native-libraries`.

## `mfb-nir` JSON (`mfb build --nir`)

`NirModule::to_json` emits the stable, versioned debug form. The envelope is
fixed at `"format": "mfb-nir"`, `"version": 1`. The `globals` key is omitted
entirely when there are no globals. Source locations and the `For` header `loc`
are intentionally dropped; `Capture.byRef` is emitted only when true.
[[src/target/shared/nir/json.rs:to_json]]

```json
{
  "format": "mfb-nir",
  "version": 1,
  "target": "macos-aarch64",
  "buildMode": "console",
  "project": "demo",
  "entry": { "name": "main", "returns": "Nothing", "accepts_args": false },
  "globals": [
    { "name": "counter", "symbol": "_mfb_global_demo_counter",
      "visibility": "private", "mutable": true, "type": "Int", "value": null }
  ],
  "types": [
    { "kind": "type", "visibility": "public", "name": "Point",
      "fields": [ { "visibility": null, "name": "x", "type": "Int" } ] }
  ],
  "imports": [
    { "package": "link", "name": "libm.cbrt", "symbol": "_mfb_linker_libm_cbrt",
      "kind": "func", "isolated": false, "params": [], "returns": "" }
  ],
  "runtimeHelpers": ["io", "strings"],
  "functions": [
    { "name": "main", "visibility": "public", "kind": "sub",
      "isolated": false, "params": [], "returns": "Nothing",
      "body": [
        { "op": "eval", "value": {
            "kind": "runtimeCall", "helper": "io",
            "target": "io.print", "args": [
              { "kind": "const", "type": "String", "value": "hi" } ] } }
      ] }
  ]
}
```

`"buildMode"` reflects `NativeBuildMode::as_str()` (`console` or `macos-app`).
`union` types serialize with `includes`/`variants` and `enum` types with
`members` instead of `fields`. [[src/target/shared/nir/json.rs:to_json]]

## See Also

- `./mfb spec architecture ir` — the shared IR that NIR lowers from
- `./mfb spec architecture monomorphization` — source of `_mfb_fn_` callee names
- `./mfb spec architecture native` — per-target code generation downstream of NIR
- `./mfb spec architecture artifacts` — the `--nir` artifact among build outputs
- `./mfb spec memory runtime-helper-abi` — `_mfb_rt_*` runtime-call ABI
- `./mfb spec memory fallible-call-abi` — `CallResult` calling convention
- `./mfb spec memory native-calling-convention` — register/stack rules for calls
- `./mfb spec linker symbols-and-relocations` — how NIR symbols are emitted
- `./mfb spec linker package-linking` — `LINK` thunks and `_mfb_linker_init`
- `./mfb spec language native-libraries` — `LINK` source-level semantics
- `./mfb spec package ir-section` — encoded IR carried in packages, merged pre-NIR
