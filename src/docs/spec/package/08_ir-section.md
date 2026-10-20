# Binary Representation Section

The `IR` section (id `16`) carries the structured Binary Representation payload — the faithful, versioned serialization of the project's IR functions.

## Payload header

```text
magic        4 bytes = "MFBR"
version      u16
IrProject    ...
```

Required `magic`:

```text
4D 46 42 52
M  F  B  R
```

The Binary Representation `version` is **`7`**. The decoder requires `MFBR` magic and exactly this version, independently of MFPC major version `2`. Version 7 spells the internal fallible result `#Result OF T` in every inline type string (a v6 `Result OF T` would read as a user template, so v6 is refused). [[src/ir/binary/mod.rs:BINARY_REPR_VERSION]] [[src/ir/binary/mod.rs:decode_binary_repr]]

Statements, case arms, and declarations carry source locations as `u32` line and column. A binding encodes its location, declaring `file`, then `explicit_type`; a type declaration encodes its location then `file`. A `Bind` statement writes `explicit_type` immediately before its trailing location. `explicit_type` distinguishes a source annotation from an inferred type. [[src/ir/binary/mod.rs:encode_binding]] [[src/ir/binary/mod.rs:encode_type]] [[src/ir/binary/op_codec.rs:encode_op]]

A function encodes its source file and location, followed by a `u32`-counted ownership map sorted by binding name. Each entry has a name string and an owner tag: `0` = `Local`, `1` = `Float` followed by the owning binding name. These ownership decisions accompany decoded package IR; they are not re-derived from source syntax. [[src/ir/binary/mod.rs:encode_function]] [[src/ir/binary/mod.rs:decode_resource_owners]]

The payload is self-contained: integers are little-endian, strings are inline length-prefixed (`u32` byte length followed by UTF-8 bytes). Crucially, the payload does **not** reference the container's `STRING_POOL` or other interned tables — every name and type inside the `IrProject` is written inline, so a function body is fully reconstructable from this payload alone. The container's metadata sections (type table, const pool, function table, exports, ABI, …) are a **parallel, derived** view used for fast scanning, ABI compatibility, and identity checks; the consumer reconstructs executable IR from the `MFBR` payload, not from those tables. The in-memory IR is free to change behind this format; the encoding is the stable contract, and carries executable declarations and bodies; documentation and native-library locators are stored in separate sections. [[src/ir/binary/mod.rs:decode_project]]

## Payload structure (`IrProject`)

`encode_project`/`decode_project` lay the project out as: [[src/ir/binary/mod.rs:encode_project]]

```text
name            str
entry           u8 present-flag, then if present: name str, returns str, acceptsArgs bool
bindings        vec<Binding>     (top-level LET/MUT, with initializer values)
types           vec<Type>
functions       vec<Function>    (each with its full structured body)
--- optional native LINK trailer (see native-bindings) ---
linkFunctions   vec<IrLinkFunction>
linkAliases     vec<(alias str, target str)>
linkCstructs    vec<(alias str, name str, mapsTo str, fields vec<(name str, ctype str)>)>
--- optional STATE trailer (see native-bindings) ---
linkState       vec<(alias str, name str, returnStateType optional str, bindState optional str)>
```

`vec<T>` is a `u32` count followed by that many elements. The native trailer is emitted when any of `linkFunctions`, `linkAliases`, or `linkCstructs` is non-empty. End-of-buffer after `functions` means there is no native trailer. The STATE trailer is emitted only when a native function has a return state type or a state binding. [[src/ir/binary/mod.rs:encode_project]] [[src/ir/binary/link_codec.rs:encode_link_state_trailer]]

## Structured control flow (no jumps)

Control flow is encoded as nested regions with explicit ends, matching IR exactly:

```text
IF      <cond-expr> THEN <ops...> ELSE <ops...> END
WHILE   <cond-expr> DO <ops...> END
DO      <ops...> UNTIL <cond-expr>
FOR     <name> = <start> TO <end> STEP <step> DO <ops...> END
FOREACH <name> IN <iterable-expr> DO <ops...> END
MATCH   <scrutinee-expr> CASE <pattern> [<guard>] <ops...> ... [ELSE <ops...>] END
TRAP    <binding> <ops...> END
```

Structured exit out of these regions is itself encoded as leaf ops rather than jumps: `ExitLoop` (`EXIT FOR/DO/WHILE`), `ContinueLoop` (`CONTINUE FOR/DO/WHILE`), and `ExitProgram` (`EXIT PROGRAM`). There are no `JMP`, `JMP_FALSE`, label, or program-counter concepts in the format. A reader walks the tree; structure is read, never reconstructed.

## Statements / ops

`IrOp` is encoded faithfully, one tag byte per kind (`encode_op`). The current tag assignment is: [[src/ir/binary/op_codec.rs:encode_op]]

```text
0  = Bind            1  = Assign          2  = AssignGlobal
3  = Return          4  = Fail            5  = Eval
6  = If              7  = Match           8  = While (conditional)
9  = ForEach        10  = Trap           11  = ExitLoop
12  = ContinueLoop  13  = ExitProgram    14  = For
15  = DoUntil       16  = While (unconditional / loop forever)
17  = StateAssign    (thread resource state assignment)
```

Every op ends with its `loc` (trailing `u32` line + `u32` column — the source statement's span). A `Bind` op additionally writes its `explicit_type` bool immediately before that `loc`. Source-level `PROPAGATE` and `RECOVER` are lowered before serialization (`PROPAGATE` becomes `Fail`; `RECOVER` is lowered into ordinary ops), so they are not distinct Binary Representation ops. There are no `RESOURCE_ENTER`/`LEAVE`/`CLOSE` ops: cleanup is reconstructed from typed scopes and encoded ownership decisions (see ./mfb spec package resource-regions). The internal `#Result`/`#Ok` forms remain implementation-only — they appear in IR and therefore in Binary Representation, but are never user-visible.

## Expressions stay nested

`IrValue` is encoded as a tree, with one tag byte per kind and kind-specific fields. `Call` and `CallResult` carry a target and arguments; `Checked` carries a success type and a call-free value whose local errors it captures. `SetLiteral` uses tag `21`; an observing `Checked` uses tag `22` and a capture-only `Checked` (`observe` FALSE) uses tag `23`, so a package with no capture-only node encodes exactly as before that tag existed. Unknown tags are rejected. Built-in calls use the same call nodes as other functions. [[src/ir/binary/value_codec.rs:encode_value]] [[src/ir/binary/value_codec.rs:decode_value_body]]

Computed IR nodes carry canonical type-name annotations: calls, checked expressions, arithmetic, member access, and extracted result values encode their result types. Locals and globals resolve through declarations. These annotations and declaration fields let the semantic checker validate decoded IR without source syntax, but they remain untrusted input and checks may skip types they cannot reconstruct. [[src/ir/binary/value_codec.rs:encode_value]] [[src/ir/verify/compat.rs:check_call_result_type]]

## Tables and references

Unlike the container metadata sections, the `MFBR` payload does **not** reference declarations by index into the container's interned tables. Names and types inside IR nodes are written inline as strings (type references are canonical type-name strings such as `"List OF Integer"` or `"#Result OF Out"`, the same canonical names the `TYPE_TABLE` interns). The `TYPE_TABLE`, `CONST_POOL`, and friends are derived from the same IR for scanning and ABI purposes, but the payload itself carries everything needed to rebuild the IR standalone. Executable IR decoding does not consult the other sections. [[src/ir/binary/mod.rs:decode_project]]

## Consumption

A consumer decodes each package IR, structurally verifies it, prefixes package symbols with `<id>.package.symbol`, merges definitions and references into the importing project, orders global initialization, and runs the semantic verifier before native lowering. [[src/target/shared/nir/lower.rs:merge_packages]]

`<id>` is a **deterministic content hash** — never a per-build random value. Concretely (`package_identity_id`): it is the first **8 bytes of SHA-256**, rendered as **16 lowercase hex characters**, hashed over the package identity (`name`, `version`, `ident`, each prefixed by its `u64` length) followed by the entire MFPC `packageBinaryRepr` (all sections), not just this inner `MFBR`/IR payload. [[src/binary_repr/reader.rs:package_identity_id]] Because it is content-addressed, the same package reached through two dependency paths produces the same `<id>` and de-duplicates to a single merged copy, while two distinct packages that happen to share a name receive different `<id>`s and stay separate instead of colliding. The prefix is applied by the *consumer* at merge time as a consistent rename of the package's definitions **and** of every reference to them (from the executable and from other packages).

## See Also

* ./mfb spec package binary-representation — the section table this `IR` payload sits in
* ./mfb spec architecture binary-representation — the structured node model encoded here
* ./mfb spec package functions — the function entries whose bodies this section carries
