# IR Lowering

The typed, architecture-independent IR: the operation and value forms shared by both back ends.

IR lowering is implemented in `src/ir.rs`.

The IR is a typed, architecture-independent representation of the concrete AST.
It contains:

- Project name.
- Optional executable entry point.
- Top-level bindings (program globals).
- User-defined types.
- Functions.
- Parameters and defaults.
- Structured operations.
- Structured expression values.

The main IR operation forms are:

- `Bind`
- `Assign`
- `AssignGlobal` — assignment to a top-level binding
- `Return`
- `ExitLoop` — structured `EXIT FOR/DO/WHILE` (carries the `LoopKind`)
- `ContinueLoop` — structured `CONTINUE FOR/DO/WHILE` (carries the `LoopKind`)
- `ExitProgram` — `EXIT PROGRAM` with a status code and full RAII unwind
- `Fail`
- `Eval`
- `If`
- `Match`
- `While` — `WHILE`/`DO WHILE` loops (carries the `LoopKind`)
- `For` — counted `FOR` loops with start/end/step
- `DoUntil` — bottom-tested `DO ... UNTIL` loops
- `ForEach`
- `Trap` — inline `TRAP` block; `RECOVER` is lowered into the trap body via
  recover targets rather than a dedicated op

The main IR value forms are:

- `Const` — typed literal constants
- `Local` — local variable references
- `Global` — top-level binding references
- `FunctionRef` — references to named functions
- `Closure` — closure values with a name, type, and captured variable list
- `Capture` — reference to a captured variable by index inside a closure body
- `Call` — calls to functions that return a plain value
- `CallResult` — calls to functions that return `Result<T>`
- `Constructor` — record/struct constructors
- `UnionWrap` — wrapping a value as a specific union member
- `UnionExtract` — extracting the inner value from a union member
- `ResultIsOk` — testing whether a `Result` value is the success case
- `ResultValue` — extracting the success value from a `Result`
- `ResultError` — extracting the error value from a `Result`
- `WithUpdate` — record update expressions
- `ListLiteral` — list literal values
- `MapLiteral` — map literal values
- `MemberAccess` — member field access
- `Binary` — binary operator expressions
- `Unary` — unary operator expressions

`mfb build -ir` serializes this representation to `<project>.ir`.

IR is intentionally shared by both downstream products:

- Native executable generation lowers IR to target-specific native structures.
- Package generation lowers IR to architecture-independent MFPC binary representation.
