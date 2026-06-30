# The .ir Debug Artifact

The human-readable JSON dump of the typed IR, emitted by `mfb build -ir`.

`mfb build -ir` lowers the concrete AST to an `IrProject`, then writes
`<name>.ir` next to the project (the file is named from the project name, not the
flag).[[src/cli/build.rs:BuildOutput]][[src/ir/lower.rs:write_ir]] The contents are
`IrProject::to_json`, a hand-rolled pretty-printer.[[src/ir/json.rs:to_json]]

This is a DISTINCT, separately versioned debug serialization. It is **not** the
MFBR binary wire format that ships inside `.mfp` packages (see the `package
ir-section` topic and the `architecture binary-representation` topic). The two
formats have independent version numbers and may diverge freely. The `.ir` dump
is lossy and for human inspection only; nothing reads it back.

## Header

Every dump is a single JSON object opening with a fixed header:[[src/ir/json.rs:to_json]]

```json
{
  "format": "mfb-ir",
  "version": 1,
  "project": "...",
  "entry": { ... } | null,
  "bindings": [ ... ],
  "types": [ ... ],
  "functions": [ ... ]
}
```

- `project` is the project name string.
- `entry` is the optional executable entry point, or `null` for a package.
- `bindings` is **omitted entirely** when there are no top-level bindings (not
  emitted as an empty array).[[src/ir/json.rs:to_json]]
- `types` and `functions` are always present (possibly empty).

Indentation is produced by manual padding; `join_json` separates array elements
with a bare comma and each node prepends its own newline + pad.[[src/ir/json.rs:join_json]]
The output is conventional JSON; strings are escaped by the shared
`json_string` helper.[[src/main.rs:json_string]]

## Node shapes

### entry[[src/ir/mod.rs:EntryPoint]]

```json
{ "name": "...", "returns": "...", "accepts_args": true }
```

`returns` is a type-name string; `accepts_args` is a bare boolean.

### bindings[[src/ir/types.rs:IrBinding]]

Top-level program globals, one object per binding:

```json
{ "name": "...", "visibility": "...", "mutable": false, "type": "...", "value": <value> | null }
```

`value` is a value node (below) or `null`.

### types[[src/ir/types.rs:IrType]]

The shape is selected by `kind`, one of `"type"`, `"union"`, `"enum"`. An
unknown kind is `unreachable!`.

| `kind` | Fields |
| --- | --- |
| `type` | `kind`, `visibility`, `name`, `fields` (array of field nodes) |
| `union` | `kind`, `visibility`, `name`, `includes` (array of strings), `variants` (array of variant nodes) |
| `enum` | `kind`, `visibility`, `name`, `members` (array of member nodes) |

Field node:[[src/ir/types.rs:IrField]]

```json
{ "visibility": "..." | null, "name": "...", "type": "..." }
```

Variant node (union):[[src/ir/types.rs:IrVariant]]

```json
{ "name": "...", "fields": [ <field>, ... ] }
```

Enum member node:[[src/ir/types.rs:IrEnumMember]]

```json
{ "name": "..." }
```

### functions[[src/ir/mod.rs:IrFunction]]

```json
{
  "name": "...",
  "visibility": "...",
  "kind": "...",
  "params": [ <param>, ... ],
  "returns": "...",
  "body": [ <op>, ... ]
}
```

Param node:[[src/ir/types.rs:IrParam]]

```json
{ "name": "...", "type": "...", "default": <value> | null }
```

## Ops

Each op node carries an `"op"` discriminator string.[[src/ir/op.rs:IrOp]] The
emitted name differs from the Rust variant name where noted.

| `op` | Variant | Payload keys |
| --- | --- | --- |
| `bind` | `Bind` | `mutable` (bool), `name`, `type`, `value` (value \| `null`) |
| `assign` | `Assign` | `name`, `value` |
| `assignGlobal` | `AssignGlobal` | `name`, `value` |
| `stateAssign` | `StateAssign` | `resource`, `value` |
| `return` | `Return` | `value` (value \| `null`) |
| `exitLoop` | `ExitLoop` | `loop` (loop-kind name) |
| `continueLoop` | `ContinueLoop` | `loop` |
| `exitProgram` | `ExitProgram` | `code` (value) |
| `fail` | `Fail` | `error` (value) |
| `eval` | `Eval` | `value` |
| `if` | `If` | `condition`, `then` (op array), `else` (op array) |
| `match` | `Match` | `value`, `cases` (match-case array) |
| `while` | `While` | `loop`, `condition`, `body` (op array) |
| `for` | `For` | `name`, `type`, `start`, `end`, `step`, `body` |
| `doUntil` | `DoUntil` | `condition`, `body` |
| `forEach` | `ForEach` | `name`, `type`, `iterable`, `body` |
| `trap` | `Trap` | `name`, `body` |

`loop` names come from `loop_kind_name`: `"for"`, `"do"`, `"while"`.[[src/ir/json.rs:loop_kind_name]]

Match-case node:[[src/ir/value.rs:IrMatchCase]]

```json
{ "pattern": <pattern>, "guard": <value> | null, "body": [ <op>, ... ] }
```

Match-pattern node, keyed by `kind`:[[src/ir/value.rs:IrMatchPattern]]

| `kind` | Payload |
| --- | --- |
| `else` | (none) |
| `value` | `value` |
| `oneOf` | `values` (array of values) |

## Values

Expression values are tagged by a `"kind"` string.[[src/ir/value.rs:IrValue]] Values
serialize inline (the `to_json` indent argument is ignored for values).

| `kind` | Variant | Payload keys |
| --- | --- | --- |
| `const` | `Const` | `type`, `value` (both strings) |
| `local` | `Local` | `name` |
| `global` | `Global` | `name` |
| `localRef` | `LocalRef` | `name`, `type` |
| `functionRef` | `FunctionRef` | `name`, `type` |
| `closure` | `Closure` | `name`, `type`, `captures` (value array) |
| `capture` | `Capture` | `index`, `type`, and `byRef: true` **only** when slot-borrowed |
| `call` | `Call` | `target`, `args` (value array) |
| `callResult` | `CallResult` | `target`, `args` |
| `constructor` | `Constructor` | `type`, `args` |
| `unionWrap` | `UnionWrap` | `union`, `member`, `value` |
| `unionExtract` | `UnionExtract` | `type`, `value` |
| `resultIsOk` | `ResultIsOk` | `value` |
| `resultValue` | `ResultValue` | `value` |
| `resultError` | `ResultError` | `value` |
| `with` | `WithUpdate` | `type`, `target`, `updates` (record-update array) |
| `list` | `ListLiteral` | `type`, `values` |
| `map` | `MapLiteral` | `type`, `entries` (each `{ "key": <value>, "value": <value> }`) |
| `memberAccess` | `MemberAccess` | `target`, `member` |
| `binary` | `Binary` | `op` (string), `left`, `right` |
| `unary` | `Unary` | `op` (string), `operand` |

Record-update node (used by `with`):[[src/ir/types.rs:IrRecordUpdate]]

```json
{ "field": "...", "value": <value> }
```

`capture` is the one variant whose key set varies: by-value captures omit
`byRef`, slot-borrow captures emit `"byRef": true`.[[src/ir/value.rs:IrValue]]

## Notes

- `version` is `1` and independent of the MFBR `BINARY_REPR_VERSION`; bump one
  without the other.[[src/ir/json.rs:to_json]][[src/ir/binary.rs:BINARY_REPR_VERSION]]
- Native back-end IR fields on `IrProject` (`native_resources`,
  `link_functions`, `link_aliases`, `docs`) are part of the in-memory model but
  are **not** serialized into the `.ir` dump.[[src/ir/json.rs:to_json]]
- The dump is write-only; the binary representation (`architecture
  binary-representation`) is the round-trippable form.

## See Also

- `./mfb spec architecture ir` — the IR model this artifact serializes
- `./mfb spec package ir-section` — the separate MFBR binary wire format
- `./mfb spec architecture binary-representation` — MFBR encoding internals
- `./mfb spec architecture artifacts` — the full build-artifact table
