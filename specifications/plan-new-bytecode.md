# Plan: Bytecode becomes a structured **Binary IR** (one codegen, no bridge)

Status: proposed (planning only — no spec or compiler changes yet)
Owner: Justin
Date: 2026-06-19
Blocks/unblocks: completes the package half of `plan-result-cleanup.md` §6 (inline
`TRAP` on a built-in inside a package export), which is wedged behind the current
bytecode design.

## 1. Terminology decree

**"Bytecode" means a *Binary IR representation*.** Everywhere the term appears —
`package_format.md`, `mfbasic.md`, `standard_package.md`, `threading.md`,
`error_codes.md`, source comments, diagnostics — it refers to a compact,
**structured** binary encoding of the compiler's IR, not a flat register/stack
machine. The `.mfp` package payload is Binary IR. There is no flat opcode ISA,
no `JMP`/`JMP_FALSE`, no register machine.

This document uses "Binary IR" and "bytecode" interchangeably from here on.

## 2. Why

### 2.1 The pipeline today

```
                       ┌──► NIR ──► native machine code     (executable's own code)
Source → AST → IR ─────┤
                       └──► flat bytecode (.mfp)            (package: stops here)
```

`NIR` and `flat bytecode` are **siblings** lowered from `IR`. They are different
levels:

- **IR** is *structured*: `If { then, else }`, `While { body }`, `Match`, `Trap`,
  nested expressions. Knows `List`, `Map`, `Result`, owned resources, threads.
- **Flat bytecode** is a *lowering* of IR to a register machine with explicit
  jumps (`compare; JMP_FALSE …`). The nesting is dissolved into jumps; nested
  expressions are broken into one-op-per-register steps.

### 2.2 The problem the lowering creates

When an executable **imports** a package, the consumer must turn the package's
bytecode into native code. Because the package is *flat bytecode*, the consumer
runs a **second, separate** bytecode→native path —
`lower_package_export_function` in `src/target/shared/code/mod.rs` — that is
distinct from the complete `IR→NIR→native` path the executable's own code uses.

That second path is **half-built**: it lowers ~18 straight-line opcodes
(`LOAD_CONST`, `ADD`, `SUB`, `NEG`, `NOT_EQUAL`, `CALL_RESULT`, `UNWRAP_RESULT`,
`RETURN_OK`, `CONSTRUCT_RECORD`, runtime-helper calls, resource enter/leave/close,
`GENERAL_LEN`, `FS_PATH_JOIN`, …). It has **no** comparisons besides `≠`, **no
branches**, **no `RESULT_IS_OK/VALUE/ERROR`**, **no `FAIL`/`PROPAGATE`**, and
almost no built-ins. So:

- A package export with a plain `IF` fails to consume (`opcode 26 OPCODE_LESS …
  not lowered by the native package bridge`).
- A package export calling `toInt` fails (`opcode 105 OPCODE_GENERAL_TO_INT …`).
- A package export that inline-`TRAP`s a built-in can't even **build** to bytecode
  today (`IR references unknown function 'toInt'` — the flat emitter's
  `CallResult` arm only knows user functions).

The existing worker/accessor packages work only because their bodies happen to be
exactly the straight-line shape the half-built bridge covers.

This is the root cause of the wedged item in `plan-result-cleanup.md`: inline
`TRAP` on a built-in works in executables (already fixed — `IR→NIR→native` handles
it) but is impossible in a package export, because packages take the crippled
bytecode→native detour.

### 2.3 Why "flat" was the wrong altitude for a package format

The package format's job is **platform-neutral distribution**. A flat register
ISA is the right shape for a *VM that executes code* or a *verifier of untrusted
binaries* — but the lowering to it discards exactly the IR-level structure the
consumer needs, forcing either a fragile bytecode→IR decompiler or a perpetual
second native codegen. A **structured binary IR** keeps the structure, so
consumption is a faithful decode that rejoins the one complete codegen.

The structured choice is not exotic: WebAssembly deliberately uses structured
control flow (`block`/`loop`/`if`/`end`, no arbitrary jumps) for the same
reasons — easy validation, no structure loss. We adopt the same principle at
**MFBASIC's own semantic level** (the encoding still knows `List`, `Result`,
owned `File`, threads), rather than lowering into WASM's lower-level numbers +
memory model (which would re-introduce the same loss and add a foreign toolchain
dependency).

## 3. The new pipeline

```
                        Source
                          │  parse / resolve / monomorphize / typecheck
                          ▼
                          IR
                          │
         build a package  ├──► Binary IR (.mfp)   ── STOP (portable, structured)
                          │
   ── if building a package, stop here ──
                          │
   prebuilt import        │
   .mfp (Binary IR) ─────►┤  decode → IR functions, merge into the project IR
                          │
                          ▼
                         NIR
                          │
                          ▼
                 native machine code
                          │
                          ▼
                       linked .out
```

- **Package build:** `IR → Binary IR (.mfp)`. A faithful structured serialization
  of IR. No lowering, no flattening, no structure loss.
- **Executable build (own code):** `IR → NIR → native`. Unchanged.
- **Consuming a package:** **decode** the `.mfp`'s Binary IR back into IR
  functions, **merge** them into the project's IR set, then lower **everything**
  through the single `IR → NIR → native` path.
- **`lower_package_export_function` and every `lower_package_*` helper are
  deleted.** There is exactly one native codegen. Packages automatically get
  every language feature the executable path has — control flow, function-level
  and inline `TRAP`, all built-ins, inline-`TRAP`-on-built-in — for free, because
  they ride the same path (including the existing native inline-`TRAP`-on-built-in
  fix from `plan-result-cleanup.md`).

### 3.1 What is preserved

The `.mfp` **container** is unchanged in spirit: header, signature/trust, manifest,
string pool, type table, const pool, import table, export table, global table,
ABI index, resource table, native-link table. Signing, ABI hashing, and
dependency resolution operate on the payload bytes exactly as before. **Only the
encoding of a function *body* changes** — from a flat opcode stream to a
structured Binary IR node tree. Tables that exist to *describe* functions/types/
constants/strings stay; IR nodes reference them by id.

## 4. The Binary IR encoding

The encoding mirrors `src/ir.rs` (`IrProject`, `IrFunction`, `IrOp`, `IrValue`,
`IrType`, …) one-to-one. It is a **defined, versioned binary format** — *not* a
raw struct dump of compiler internals. The compiler serializes its in-memory IR
*to* this format and reads it *back*; the format is the stable contract, the
in-memory IR is free to change behind it.

### 4.1 Structured control flow (no jumps)

Control-flow ops are encoded as **nested regions with explicit ends**, matching
IR exactly:

```
IF      <cond-expr> THEN <ops...> ELSE <ops...> END
WHILE   <cond-expr> DO <ops...> END
FOREACH <name> IN <iterable-expr> DO <ops...> END
MATCH   <scrutinee-expr> CASE <pattern> <ops...> ... [ELSE <ops...>] END
TRAP    <binding> <ops...> END
```

There are no `JMP`, `JMP_FALSE`, label, or program-counter concepts in the
format. A reader walks the tree; structure is read, never reconstructed.

### 4.2 Expressions stay nested

`IrValue` is encoded as a tree (`Binary { op, left, right }`, `Call { target,
args }`, `CallResult { … }`, `ResultIsOk/Value/Error`, `Constructor`,
`MemberAccess`, `UnionWrap/Extract`, literals, identifiers, …). No
flattening into per-register temporaries. (`CallResult` of a built-in is just an
`IrValue::CallResult` node — the flat emitter's "unknown function" failure does
not exist, because nothing dispatches built-ins to function ids.)

### 4.3 Statements / ops

`IrOp` is encoded faithfully: `Let`/`Bind`, `Assign`, `Return`, `Fail`,
`Propagate`, `Recover`, `Eval`, plus the control-flow ops above, plus the
resource region ops (`RESOURCE_ENTER`/`LEAVE`/`CLOSE`). The internal `Result`/`Ok`
forms remain implementation-only (per `plan-result-cleanup.md`): they appear in IR
and therefore in Binary IR, but are never user-visible.

### 4.4 Tables and references

Reuse the existing interned tables: strings, types (concrete instantiations such
as `List OF Integer`, `Result OF Out`), constants, globals, imports, exports.
IR nodes reference entries by id. The function table records name, signature,
params (with defaults), declared return/effect, ownership annotations, and the
offset/length of the function's Binary IR body.

### 4.5 Versioning and migration

- Bump the package/bytecode format version. **Clean break, by decision:** the
  structured Binary IR is the *only* payload the reader accepts. The old flat
  payload is rejected outright — no dual-read, no migration, no backward
  compatibility. (No external `.mfp` packages exist; in-repo fixtures are
  regenerated.)
- All committed `tests/**/packages/*.mfp` fixtures are **regenerated** from their
  sources as part of this change. (If sources for a fixture are not in-repo,
  reconstruct minimal equivalents; every worker/accessor package used by tests is
  simple enough to rebuild.)

### 4.6 Package identity prefix

Every package carries a **package identity ID** in its `.mfp` header, and that ID
namespaces all of the package's symbols at link/merge time:

```
<id>.<package>.<symbol>      e.g.  7f3a…c901.json.parse
```

applied to functions, types, constants, and globals. This is how the cross-package
IR merge (§7) keeps symbols from colliding and how it resolves inter-package
references — it replaces the bridge's ad-hoc `package_*` symbol/offset plumbing with
one uniform rule.

**The ID is deterministic, derived from package identity — never random.** It is a
hash of the package's identity (name + version + content/ABI), reusing the existing
`ABI_INDEX` / `IMPORT_TABLE` version-pin + ABI-hash machinery rather than inventing
a parallel scheme. A random per-compile ID is explicitly rejected because it would
break:

- **Reproducible builds** — the same source must produce the same `.mfp` bytes (for
  caching, content-addressing, signature stability, and `.hex`/golden diffs).
- **Dependency de-duplication** — in a diamond graph (app → A → C and app → B → C,
  same C), a deterministic identity gives C **one** ID via both paths, so the merge
  keeps a single copy and both references resolve to it. A random ID would link two
  copies of the same C, or leave references unresolved.
- **Cross-package references** — a reference baked against a per-build random ID
  welds the dependent to one specific build; a content/identity ID stays stable
  across rebuilds that don't change the package.

Identical content reached via two dependency paths shares one ID and de-duplicates;
differing content yields differing IDs and surfaces a version conflict explicitly.

**Application is a link-time rename.** The prefix is applied by the *consumer* at
merge/link time, as a consistent rename of the package's definitions **and** of
every reference to it (from the executable and from other packages), driven by the
resolved dependency graph — not baked at each package's own build time. Inside a
`.mfp`, references to *other* packages stay logical (import name + the resolved ABI
identity, already recorded in `IMPORT_TABLE`/`ABI_INDEX`); the consumer resolves
them to the concrete `<id>` prefix during the merge.

## 5. Verification

Verification moves from "verify a flat opcode stream" to "verify decoded IR." The
invariants in `package_format.md` (type-correctness at every point, resource
linearity / no double-drop / use-after-move, exhaustive `MATCH`, single bottom
trap, no error-routing via unwinding, declared return/effect agreement, native-link
validity) are re-stated as **IR-level invariants** and checked on the decoded
package IR before it is merged.

Benefit: the structured form is *easier* to verify (structure is explicit — no CFG
reconstruction, no "reject jumps into trap/cleanup regions"), and much of the
analysis can **share the compiler's existing IR-level passes** (ownership/resource
checking, exhaustiveness, type agreement) rather than maintaining a parallel
flat-bytecode verifier.

## 6. Completing the blocked `plan-result-cleanup.md` work

The package half of inline-`TRAP`-on-built-in is resolved *for free* by this
redesign and needs no special code:

- **Package build:** `IR::CallResult{ built-in }` is serialized as an ordinary IR
  node. The flat emitter's "unknown function `toInt`" failure cannot occur — there
  is no flat built-in dispatch.
- **Consumption:** the decoded IR flows through `IR→NIR→native`, which already
  lowers inline-`TRAP`-on-built-in (the native fix landed in
  `plan-result-cleanup.md` task 8: helper-backed `CallResult` → raw `Result` →
  `materialize_current_result`).

So once this plan lands, `plan-result-cleanup.md` tasks 9 (bytecode emitter
`CallResult` of built-ins) and 10 (bytecode→native bridge) are **obsolete/subsumed**,
and task 11 (end-to-end package test) becomes the acceptance test for *this* plan
(see §9).

## 7. Compiler changes (`src/`)

- **`src/ir.rs`** — add a versioned binary **encode** (`IR → Binary IR bytes`) and
  **decode** (`Binary IR bytes → IR`) for `IrProject`/`IrFunction`/`IrOp`/`IrValue`/
  `IrType` and the supporting tables. (Mechanical: one case per node kind, both
  directions. The existing JSON `write_ir` is the reference for field coverage.)
- **`src/bytecode.rs`** — **replace** the flat emitter. `encode_*` for the flat
  opcode stream, the opcode constants, `FunctionBuilder`'s lowering of `IrOp`/
  `IrValue` to opcodes, and the flat reader are **removed**. `bytecode.rs` becomes
  the container reader/writer (header, sections, signing, ABI, tables) plus the
  Binary IR payload from `ir.rs`. The flat `NativeInstruction`/`NativePackageExport`
  types are removed.
- **`src/target/shared/code/mod.rs`** — **delete** `lower_package_export_function`,
  `lower_package_runtime_call`, `lower_package_call_result`,
  `lower_package_raw_result_store`, `lower_package_equality`,
  `lower_package_construct_record`, `lower_package_return_union_variant`,
  `package_runtime_symbol`, and the rest of the straight-line bridge. Replace the
  package-consumption entry with: decode each `.mfp`'s Binary IR → IR functions →
  merge into the module that goes to `IR→NIR→native`.
- **`src/target/shared/nir.rs`, `plan.rs`, `code/builder_*.rs`** — **unchanged**;
  they are the one codegen and now serve package functions too.
- **`src/main.rs`** — `build_project`:
  - package build: `IR → Binary IR` via `target::write_package` (now an IR
    serialize + container write).
  - executable build: read each package's Binary IR, decode to IR, merge into the
    project IR (namespacing package symbols/types/constants/globals as the bridge
    did, but at IR level), then `IR→NIR→native` as today.
  - `-bc` output: dump the Binary IR (structured) instead of flat opcodes.
- **Package IR merge** — a small, well-scoped pass: bring a package's IR functions
  in under their **identity-prefixed** names (`<id>.pkg.func`, §4.6), merge its
  type/const/global tables into the project's, and resolve cross-package references
  to the concrete prefixes from the resolved dependency graph. This replaces the
  bridge's ad-hoc `package_string_symbols`/`package_global_offsets`/
  `package_native_exports` machinery with one IR-level merge.

## 8. Spec changes (`specifications/`)

- **`package_format.md`** — the big one. Rewrite from "typed register bytecode +
  opcode tables + flat call/result/jump ops + cleanup tables/`trapPc`" to
  "structured Binary IR payload." Specifically:
  - Replace the opcode/ISA sections, the `CALL_RESULT`/`UNWRAP_RESULT`/`MAKE_OK`/
    `RESULT_IS_OK`/`JMP`-style listings, and the flat "Function calls and Result",
    "Errors and traps", "Resource cleanup", and "Cleanup Table" encodings with the
    structured IR node encoding (§4 here).
  - Keep and re-target the container, sections, manifest, string/type/const/import/
    export/global tables, ABI index, signing/trust, and native-link metadata to
    reference Binary IR bodies.
  - Re-state the verifier invariants as IR-level checks (§5).
  - Bump the format version; state the clean break.
- **`mfbasic.md`** — §15 "bytecode invariants" and any prose that describes
  bytecode as a register/opcode machine: reword to "Binary IR." The §8.8
  desugaring stays (it documents IR-level lowering and is already implementer-
  facing); ensure it reads as IR, not opcodes.
- **`standard_package.md`** — any "bytecode"/"monomorphized before bytecode
  generation"/register-machine references → Binary IR phrasing.
- **`threading.md`** — references to bytecode-level call/Result lowering and worker
  function encoding → Binary IR phrasing; worker functions are ordinary IR carried
  in the package and lowered via `IR→NIR→native` like any function.
- **`error_codes.md`** — verifier/toolchain diagnostics that name flat-bytecode
  conditions (e.g. "opcode … not lowered", "jump into trap region") are removed or
  re-stated as IR-level verification diagnostics. Add any new decode/verify
  diagnostics (`PACKAGE_BINARY_IR_VERSION_UNSUPPORTED`, `PACKAGE_IR_DECODE_FAILED`,
  `PACKAGE_IR_VERIFY_*`).
- **`plan-result-cleanup.md`** — mark the package half (tasks 9/10) as subsumed by
  this plan; point its task 11 (end-to-end package test) here.
- Sweep all four reference specs so **"bytecode" consistently means Binary IR**
  per §1.

## 9. Testing / validation

- **End-to-end package test (the acceptance test):** a package whose exported
  function contains real control flow **and** inline-`TRAP`s a built-in, consumed
  by an executable that imports and calls it. Build the `.mfp`, build the
  executable, **run it**, assert behavior on both the success and trapped-error
  paths. This is the test that is impossible today and that this plan exists to
  enable.
- **Regression coverage:** regenerate every `tests/**/*.mfp` fixture; the full
  acceptance suite (`scripts/test-accept.sh`) must pass, including all existing
  thread/worker/package tests now flowing through `IR→NIR→native`.
- **Round-trip test:** `IR → Binary IR → IR` is identity for a representative
  corpus (every `IrOp`/`IrValue`/`IrType` kind), proving the encode/decode is
  faithful.
- **`-bc` golden:** structured Binary IR dump goldens for a few packages.
- **Verifier tests:** malformed/version-mismatched/ill-typed Binary IR is rejected
  with the new diagnostics.

## 10. Sequencing

1. **Encode/decode + round-trip** (`ir.rs`): land `IR ↔ Binary IR` with the
   identity round-trip test. No behavior change yet.
2. **Container swap** (`bytecode.rs`, `write_package`): `.mfp` carries Binary IR;
   delete the flat emitter. Regenerate fixtures. `-bc` goldens.
3. **Consumption via the one path** (`main.rs`, `code/mod.rs`): decode + IR merge
   + `IR→NIR→native`; delete `lower_package_export_function` and the
   `lower_package_*` bridge. Acceptance suite green.
4. **Verifier on IR** (§5) + diagnostics.
5. **End-to-end package inline-`TRAP`-on-built-in test** (§9); close
   `plan-result-cleanup.md` tasks 9–11.
6. **Spec sweep** (§8): `package_format.md` rewrite + the "bytecode = Binary IR"
   terminology pass across all specs.

Each step keeps the tree building and the acceptance suite green (steps 1–2 are
additive; step 3 swaps consumption and deletes the bridge atomically with the
fixture regeneration).

## 11. Risks / open questions

- **Cross-package IR merge.** Symbol collisions and inter-package reference
  resolution are handled by the **package identity prefix** (§4.6): a deterministic
  identity-hash `<id>` namespaces each package's functions/types/constants/globals,
  applied as a consistent link-time rename of definitions and references. The
  remaining integration work is the merge pass itself — bringing decoded package IR
  in under those prefixes, merging the type/const/global tables, and resolving
  logical inter-package references to concrete prefixes from the resolved dependency
  graph. It replaces, at a higher level, the bridge's ad-hoc `package_*`
  symbol/offset plumbing.
- **Verification parity.** Re-stating the flat-bytecode verifier as IR-level checks
  must preserve the same guarantees (resource linearity, exhaustiveness, type
  agreement). Reusing existing IR passes is the intended path; confirm coverage.
- **Format-version break.** Clean break, by decision — no backward compatibility,
  no dual-read, no migration. The reader accepts only the structured Binary IR.
  (No external `.mfp` packages exist; in-repo fixtures are regenerated.)
- **`-bc` consumers.** `-bc` still emits a `.hex` dump — it's just the hex of the
  Binary IR bytes instead of the old flat-opcode bytes. The output mechanism is
  unchanged; only the bytes being dumped differ. In-repo `.hex` goldens are
  regenerated.
- **Future VM.** A VM is explicitly out of scope and *not* foreclosed: a future VM
  either interprets the Binary IR directly (it is structured and typed) or lowers
  it through the same `IR→NIR→native` path. Nothing here removes that option.

## 12. Non-goals

- No new language semantics; no user-visible behavior change. This is an
  internal/format change plus the package-feature unblock.
- No WASM adoption (wrong altitude — would re-lower IR into a lower-level model and
  re-introduce the round-trip loss; see §2.3).
- No VM implementation (kept possible, not built).
