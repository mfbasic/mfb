# Compiler Diagnostics

Every diagnostic the compiler emits is backed by a static `Rule` drawn from a
single registry. A rule binds four immutable fields — a numeric `code`, a
SCREAMING_SNAKE_CASE symbolic `name`, a `Severity`, and a fixed `message`
template — so that call sites name a rule by its symbolic name only, and the
code, severity, and human message are looked up centrally and rendered
uniformly. [[src/rules/mod.rs:Rule]] [[src/rules/table.rs:RULES]]

## The Rule Record

A `Rule` is four `&'static str`/enum fields; the registry `RULES` is a flat
`&[Rule]` slice. [[src/rules/mod.rs:Rule]] [[src/rules/table.rs:RULES]]

| field | type | role |
| --- | --- | --- |
| `code` | `&'static str` | the `G-SSS-EEEE` numeric code (display/sort key) |
| `name` | `&'static str` | the symbolic lookup key used at every call site |
| `severity` | `Severity` | `Error`, `Warn`, or `Info` |
| `message` | `&'static str` | the fixed, parameter-free message template |

The **symbolic name is the real primary key.** A call site passes only a rule
name; lookup linear-scans the registry for the entry whose `name` matches and
returns it, falling back to a synthetic `0-000-0000 UNKNOWN_RULE` error rule
when no name matches. In a release build a missing rule degrades to that generic
error; a debug build asserts (panics) instead, so the emit-site/table drift is
caught by tests rather than shipped. [[src/rules/mod.rs:rule_for]] The `code` is therefore a stable display label,
not the lookup key, which is why two rules may legitimately carry the same code
as long as their names differ (see *Code Collisions*). Registration is likewise
independent of emission: lookup is a pure name scan, nothing requires a
registered rule to have a live call site, and the registry may carry rules
ahead of (or after) the code that emits them. [[src/rules/mod.rs:rule_for]]

## Severity

Three severities, in descending order of consequence; `Severity` implements
`Display` as the lowercase word shown in the diagnostic header. [[src/rules/mod.rs:Severity]]

| variant | rendered | meaning |
| --- | --- | --- |
| `Error` | `error` | compilation/orchestration cannot proceed |
| `Warn` | `warn` | accepted but suspect (e.g. unknown `project.json` kind) |
| `Info` | `info` | positive/confirmatory note (e.g. validation passed) |

Severity is fixed per rule in the table — it is not raised or lowered at the
call site. The unknown-rule fallback is always `Error`. [[src/rules/mod.rs:rule_for]]

## The `G-SSS-EEEE` Code Scheme

Each code is three dash-separated numeric fields: **`G-SSS-EEEE`** — a one-digit
**group** `G`, a three-digit **subsystem** `SSS`, and a four-digit
**error number** `EEEE`. The fields partition the diagnostic namespace
hierarchically: the group is the broad compiler phase/concern, the subsystem is
the specific component within that group, and the error number is the rule's
ordinal within its subsystem. The values below are the partition **as it actually
appears in `RULES`** (verified against every entry); they are the live namespace,
not an aspirational allocation. [[src/rules/table.rs:RULES]]

### Groups (`G`)

| G | group | subsystems present |
| --- | --- | --- |
| `0` | fallback (synthetic, not in `RULES`) | `000` |
| `1` | front-end: source intake, lex, parse, DOC syntax | `100`, `101`, `102`, `103` |
| `2` | semantics: project/import/symbol/type/DOC-semantics/package/testing | `200`, `201`, `203`, `205`, `208` |
| `3` | backend: verification & target/codegen | `302`, `304` |
| `5` | linking | `500` |
| `6` | package container, lockfile, signing | `603`, `605` |

Groups `4` and `7+` are unused in the current registry; `SSS` slots within a
group are likewise sparse (group 2 jumps `200, 201, 203, 205, 208` —
`202`/`204`/`206`/`207` are unallocated). The scheme leaves room; it does not
densely fill it.
[[src/rules/table.rs:RULES]]

### Subsystems (`SSS`) and their populations

| code prefix | subsystem | rules |
| --- | --- | --- |
| `1-100` | MFBASIC source intake (read/root/overlap) | 5 |
| `1-101` | lexer | 5 |
| `1-102` | parser | 12 |
| `1-103` | DOC block structure (lexer/parser) | 4 |
| `2-200` | `project.json` validation + build orchestration | 13 |
| `2-201` | imports & symbol resolution | 18 |
| `2-203` | semantic checking (typing, ownership, native ABI) | 101 |
| `2-205` | DOC block semantics (resolver) + package metadata | 23 |
| `2-208` | test framework (assertion builtins) | 7 |
| `3-302` | verification | 1 |
| `3-304` | target/codegen support | 2 |
| `5-500` | linking | 1 |
| `6-603` | lockfile | 1 |
| `6-605` | package container / signing | 9 |
| `0-000` | `UNKNOWN_RULE` fallback (synthetic) | 1 |

`EEEE` is the per-subsystem ordinal, generally `0001`-up, but it is **not
guaranteed dense or monotonic**: subsystem `2-203` allocates a high block at
`0100`-`0102` (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, `TYPE_OVERLOAD_AMBIGUOUS`,
`TYPE_INSTANTIATION_TOO_DEEP`) after `0056`/`0058` (`0057` is unallocated), plus
later `0103`/`0104`, and `2-200`
mixes a low validation block (`0001`-`0011`) with a high orchestration block
(`0100`/`0101`). Treat `EEEE` as an opaque ordinal, never as a count.
[[src/rules/table.rs:RULES]]

### Code Collisions

Because lookup keys on `name`, the registry tolerates duplicate `code`
values. Three codes are currently shared:

- **`1-102-0010`** is both `MFB_PARSE_BLOCK_TOO_DEEP` (the parser's statement-block
  nesting cap) and `MFB_PARSE_TESTING_EXPECTED_TGROUP`.
- **`2-205-0001`** is both `PACKAGE_VERSION_UNSUPPORTED` and `DOC_UNRESOLVED`.
- **`2-205-0002`** is both `NATIVE_MANIFEST_INVALID` and `DOC_NAME_MISMATCH`.

Diagnostics for these resolve correctly because each call site names the rule
symbolically; the shared code is a display-label collision only. It is tolerated,
not endorsed — do not add to it: a new rule takes the next free code in its range.
[[src/rules/mod.rs:rule_for]] [[src/rules/table.rs:RULES]]

## Diagnostic Rendering

Two rendering forms exist, both written to **stderr**: a *located* form with a
source-context window, and an *unlocated* form for diagnostics with no source
span. [[src/rules/mod.rs:show_diagnostic]] [[src/rules/mod.rs:show_general_diagnostic]]

### Located diagnostics

A located diagnostic (a rule name, a detail message, a file, a line, and
1-based start/end columns) renders a source-context window, a caret underline,
the header line, and a detail line. [[src/rules/mod.rs:show_diagnostic]]

**Source-context window.** The file is re-read and up to three lines of context
are printed, ending at the offending line. The displayed line is clamped into
the file's line range; the window starts two lines earlier (clamped at line 1).
Each context line is printed with a right-aligned 4-column line number and a
` | ` gutter:

```text
NNNN | source line text
```

**Caret underline.** When the start column is positive and the clamped display
line equals the requested line, an underline row is printed under the gutter:
`start - 1` spaces of padding, then `end - start` carets, floored at one caret:

```text
     | ^^^^
```

Start/end are 1-based columns. If the file cannot be read, or it has no lines,
the context block and caret are skipped and only the header + detail are
emitted. [[src/rules/mod.rs:show_diagnostic]]

**Header + detail.** The header packs location, severity, code, name, and
message; the detail is the call-site-supplied detail message indented 15
spaces, so the on-screen header layout is:

```text
path/to/file.mfb:LINE severity[CODE NAME]: message-template
               detailed message
```

A full example:

```text
   3 | LET x = foo(
   4 |   1, 2,
   5 |   3)
     |          ^
main.mfb:5 error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: function call has the wrong number of arguments
               foo expects 2 arguments but 3 were supplied
```

### Unlocated diagnostics

An unlocated diagnostic (a rule name and a detail message only) is for
diagnostics with no source span (orchestration, project/package, link
failures). It drops the file context, caret, and location prefix, printing just
the header and detail:

```text
severity[CODE NAME]: message-template
               detailed message
```

[[src/rules/mod.rs:show_general_diagnostic]]

In both forms the `message` is the rule's fixed template (no interpolation);
the case-specific facts live entirely in the detail line supplied by the
caller. [[src/rules/mod.rs:show_diagnostic]] [[src/rules/mod.rs:show_general_diagnostic]]

## The Rule Registry

The complete registry follows, grouped by subsystem. All 203 `RULES` entries are
listed; none are omitted, and the synthetic `0-000-0000` fallback (not a `RULES`
member) is appended at the end. Each row is `code | NAME | severity | message`,
transcribed verbatim from the table. [[src/rules/table.rs:RULES]] Unless noted,
severity is `error`.

### `1-100` — MFBASIC source intake

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `1-100-0001` | `MFB_SOURCE_READ_FAILED` | error | MFBASIC source could not be read |
| `1-100-0002` | `MFB_SOURCE_ROOT_MISSING` | error | MFBASIC source root does not exist |
| `1-100-0003` | `MFB_SOURCE_EMPTY` | error | MFBASIC source root contains no source files |
| `1-100-0004` | `MFB_SOURCE_OUTSIDE_PROJECT` | error | MFBASIC source path resolves outside the project directory |
| `1-100-0005` | `MFB_SOURCE_OVERLAP` | error | MFBASIC source file is selected by more than one source entry |

### `1-101` — Lexer

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `1-101-0001` | `MFB_LEX_UNEXPECTED_CHARACTER` | error | lexer found an unexpected character |
| `1-101-0002` | `MFB_LEX_UNTERMINATED_STRING` | error | string literal is unterminated |
| `1-101-0003` | `MFB_LEX_INVALID_UNICODE_ESCAPE` | error | `\u{...}` string escape is malformed |
| `1-101-0004` | `MFB_LEX_MALFORMED_NUMBER` | error | numeric literal is malformed |
| `1-101-0005` | `MFB_LEX_NUMBER_OUT_OF_RANGE` | error | numeric literal is out of range |

### `1-102` — Parser

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `1-102-0001` | `MFB_PARSE_EXPECTED_EXPRESSION` | error | parser expected an expression |
| `1-102-0003` | `MFB_PARSE_INVALID_IDENTIFIER` | error | identifier is invalid |
| `1-102-0004` | `MFB_PARSE_UNEXPECTED_STATEMENT` | error | parser found an unexpected statement |
| `1-102-0005` | `MFB_PARSE_UNEXPECTED_TOKEN` | error | parser found an unexpected token |
| `1-102-0006` | `MFB_PARSE_UNTERMINATED_BLOCK` | error | parser reached end-of-file inside a block |
| `1-102-0007` | `MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING` | error | pipeline expression is missing a placeholder |
| `1-102-0008` | `MFB_PARSE_MISSING_NATIVE_SYMBOL` | error | a native LINK function must declare its native SYMBOL |
| `1-102-0009` | `MFB_PARSE_MISSING_NATIVE_ABI` | error | a native LINK function must declare its ABI signature |
| `1-102-0010` | `MFB_PARSE_BLOCK_TOO_DEEP` | error | statement block nesting is too deep (shares this code — see *Code Collisions*) |
| `1-102-0010` | `MFB_PARSE_TESTING_EXPECTED_TGROUP` | error | a TESTING block may contain only TGROUP groups |
| `1-102-0011` | `MFB_PARSE_TESTING_EXPECTED_TCASE` | error | a TGROUP may contain only TCASE cases and nested TGROUP groups |
| `1-102-0012` | `MFB_PARSE_TESTING_DESCRIPTION` | error | a TGROUP/TCASE requires a string-literal description |

### `1-103` — DOC block structure (lexer/parser)

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `1-103-0001` | `DOC_UNTERMINATED` | error | DOC block was not closed with END DOC |
| `1-103-0002` | `DOC_BAD_HEADER` | error | DOC block header must be FUNC, SUB, TYPE, UNION, ENUM, or PACKAGE |
| `1-103-0003` | `DOC_UNKNOWN_LINE` | error | DOC line keyword is not recognized |
| `1-103-0004` | `DOC_EXAMPLE_UNTERMINATED` | error | EXAMPLE block was not closed with END EXAMPLE |

### `2-200` — `project.json` validation & build orchestration

The low block (`0001`-`0013`) validates `project.json`; the high block
(`0100`/`0101`) reports orchestration failures. Note `2-200-0010` is the
registry's only `info`, and `2-200-0009` one of exactly six `warn` rules
(with `2-200-0012 PROJECT_JSON_UNKNOWN_MODE`,
`2-201-0017 PRIVATE_SHADOWS_PUBLIC`,
`2-203-0104 TYPE_INLINE_TRAP_DEAD_HANDLER`,
`2-203-0108 TYPE_MONEY_LITERAL_PRECISION`, and
`2-203-0109 MONEY_INEXACT_FLOAT_LITERAL`); every other rule is `error`.
[[src/rules/table.rs:RULES]]

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-200-0001` | `PROJECT_JSON_MISSING` | error | project.json is required |
| `2-200-0002` | `PROJECT_JSON_READ_FAILED` | error | project.json could not be read |
| `2-200-0003` | `PROJECT_JSON_PARSE_FAILED` | error | project.json is not valid JSON |
| `2-200-0004` | `PROJECT_JSON_ROOT_TYPE` | error | project.json must contain a JSON object |
| `2-200-0005` | `PROJECT_JSON_REQUIRED_FIELD` | error | project.json is missing a required field |
| `2-200-0006` | `PROJECT_JSON_FIELD_TYPE` | error | project.json field has the wrong type |
| `2-200-0007` | `PROJECT_JSON_EMPTY_FIELD` | error | project.json field must not be empty |
| `2-200-0008` | `PROJECT_JSON_EMPTY_SOURCES` | error | project.json must include at least one source entry |
| `2-200-0009` | `PROJECT_JSON_UNKNOWN_KIND` | warn | project.json kind is not recognized |
| `2-200-0010` | `PROJECT_JSON_VALID` | info | reserved; not emitted — a successful validation is silent |
| `2-200-0011` | `PROJECT_ENTRY_INVALID` | error | project entry point is invalid |
| `2-200-0012` | `PROJECT_JSON_UNKNOWN_MODE` | warn | project.json mode is not recognized |
| `2-200-0013` | `PROJECT_JSON_ICON_MISSING` | error | project.json icon path does not resolve to a readable file |
| `2-200-0014` | `PROJECT_JSON_LIBRARY_INVALID` | error | a project.json `libraries` locator is malformed, carries an unknown os/arch/libc/type token, or names a `source` that is not a bare filename |
| `2-200-0015` | `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` | error | two project.json `libraries` vendor locators declare the same `source` filename |
| `2-200-0100` | `BUILD_FAILED` | error | build failed for an unclassified orchestration reason |
| `2-200-0101` | `FMT_CHECK_FAILED` | error | one or more source files are not formatted (mfb fmt --check) |

### `2-201` — Imports & symbol resolution

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-201-0001` | `IMPORT_PACKAGE_INVALID` | error | imported package binary could not be read |
| `2-201-0002` | `IMPORT_PACKAGE_NOT_DECLARED` | error | imported package is not declared |
| `2-201-0003` | `IMPORT_PACKAGE_NOT_INSTALLED` | error | declared package is not installed |
| `2-201-0004` | `IMPORT_LOCAL_PATH_INVALID` | error | local package source must be an absolute local URL |
| `2-201-0005` | `IMPORT_PACKAGE_MANIFEST_INVALID` | error | imported package manifest is invalid |
| `2-201-0006` | `IMPORT_PACKAGE_NAME_MISMATCH` | error | imported package manifest name does not match import |
| `2-201-0007` | `IMPORT_PACKAGE_KIND_INVALID` | error | imported source package must be a package |
| `2-201-0008` | `SYMBOL_DUPLICATE_IMPORT` | error | import is declared more than once |
| `2-201-0009` | `SYMBOL_DUPLICATE_LOCAL` | error | local symbol is declared more than once |
| `2-201-0010` | `SYMBOL_DUPLICATE_TOP_LEVEL` | error | top-level symbol is declared more than once |
| `2-201-0011` | `SYMBOL_UNKNOWN_IDENTIFIER` | error | identifier could not be resolved |
| `2-201-0012` | `SYMBOL_NOT_CALLABLE` | error | symbol cannot be called |
| `2-201-0014` | `SYMBOL_UNKNOWN_IMPORT` | error | package-qualified symbol uses an unknown import |
| `2-201-0015` | `SYMBOL_UNKNOWN_TYPE` | error | type name could not be resolved |
| `2-201-0016` | `SYMBOL_RESERVED_BUILTIN_NAME` | error | function name is a reserved built-in and may not be redeclared |
| `2-201-0017` | `PRIVATE_SHADOWS_PUBLIC` | warn | PRIVATE declaration shadows a PUBLIC declaration of the same name within its file |
| `2-201-0018` | `PRIVATE_PATH_HASH_COLLISION` | error | internal: two source file paths produced the same file-scope hash |

### `2-203` — Type checking, ownership, and native ABI

The largest subsystem. It covers operator/condition/literal typing
(`0001`-`0018`), bindings and calls (`0007`-`0027`), declaration shape for
records/unions/enums/funcs (`0023`-`0046`), control-flow and `EXIT`/`CONTINUE`
(`0073`-`0081`), the ownership/move/resource model (`0055`, `0056`, `0082`-`0091`,
plus high-block `0100`), the inline-TRAP / `Result` model (`0066`-`0072`, plus
high-block `0104`), and native LINK ABI validation (`0092`-`0099`). `EEEE` skips
`0054` and `0057`, and `0100`-`0102` sit out of sequence after `0099` (see *Code
Scheme*).

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-203-0001` | `TYPE_BINARY_OPERATOR_MISMATCH` | error | binary operator operands have incompatible types |
| `2-203-0002` | `TYPE_UNARY_OPERATOR_MISMATCH` | error | unary operator operand has an incompatible type |
| `2-203-0003` | `TYPE_UNARY_OPERATOR_UNKNOWN` | error | unary operator is not recognized |
| `2-203-0004` | `TYPE_FOR_REQUIRES_NUMERIC` | error | FOR loop operands must be numeric |
| `2-203-0005` | `TYPE_FOR_STEP_ZERO` | error | FOR loop step must not be zero |
| `2-203-0006` | `TYPE_CONDITION_REQUIRES_BOOLEAN` | error | control-flow condition must be Boolean |
| `2-203-0007` | `TYPE_BINDING_MISMATCH` | error | binding initializer type does not match declared type |
| `2-203-0008` | `TYPE_ASSIGNMENT_MISMATCH` | error | assignment value type does not match binding type |
| `2-203-0009` | `TYPE_INTEGER_LITERAL_OVERFLOW` | error | integer literal is outside the Integer range |
| `2-203-0010` | `TYPE_FAIL_REQUIRES_ERROR` | error | FAIL requires an Error value |
| `2-203-0011` | `TYPE_PROPAGATE_REQUIRES_TRAP` | error | PROPAGATE requires a TRAP context |
| `2-203-0012` | `TYPE_TRAP_FALLTHROUGH` | error | TRAP path can fall through |
| `2-203-0013` | `TYPE_BYTE_LITERAL_OVERFLOW` | error | integer literal is outside the Byte range |
| `2-203-0014` | `TYPE_BYTE_LITERAL_UNDERFLOW` | error | integer literal is outside the Byte range |
| `2-203-0015` | `TYPE_FLOAT_LITERAL_OVERFLOW` | error | numeric literal is outside the Float range |
| `2-203-0016` | `TYPE_FLOAT_LITERAL_UNDERFLOW` | error | numeric literal is outside the Float range |
| `2-203-0017` | `TYPE_FIXED_LITERAL_OVERFLOW` | error | numeric literal is outside the Fixed range |
| `2-203-0018` | `TYPE_FIXED_LITERAL_UNDERFLOW` | error | numeric literal is outside the Fixed range |
| `2-203-0019` | `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` | error | lambda capture is invalid |
| `2-203-0020` | `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE` | error | binding requires a type annotation or initializer |
| `2-203-0021` | `TYPE_CALL_ARGUMENT_MISMATCH` | error | function call argument type does not match parameter type |
| `2-203-0022` | `TYPE_CALL_ARITY_MISMATCH` | error | function call has the wrong number of arguments |
| `2-203-0023` | `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` | error | constructor argument type does not match field type |
| `2-203-0024` | `TYPE_CONSTRUCTOR_ARITY_MISMATCH` | error | constructor has the wrong number of arguments |
| `2-203-0025` | `TYPE_CONSTRUCTOR_REQUIRES_RECORD` | error | record constructor syntax requires a TYPE |
| `2-203-0026` | `TYPE_DEFAULT_ARG_ORDER` | error | default parameters must be trailing |
| `2-203-0027` | `TYPE_DEFAULT_VALUE_MISMATCH` | error | default parameter value has the wrong type |
| `2-203-0028` | `TYPE_DUPLICATE_ENUM_MEMBER` | error | enum member is declared more than once |
| `2-203-0029` | `TYPE_DUPLICATE_FIELD` | error | type field is declared more than once |
| `2-203-0030` | `TYPE_DUPLICATE_VARIANT` | error | union variant is declared more than once |
| `2-203-0031` | `TYPE_ENUM_REQUIRES_MEMBER` | error | enum must declare at least one member |
| `2-203-0032` | `TYPE_FUNC_MISSING_RETURN` | error | function is missing a return value |
| `2-203-0033` | `TYPE_FUNC_REQUIRES_RETURN_TYPE` | error | FUNC must declare a return type |
| `2-203-0034` | `TYPE_FIELD_ACCESS_REQUIRES_RECORD` | error | field access requires a record value |
| `2-203-0035` | `TYPE_LET_REQUIRES_VALUE` | error | immutable binding must have an initializer |
| `2-203-0036` | `TYPE_MEMBER_NOT_VISIBLE` | error | type member is not visible from this scope |
| `2-203-0037` | `TYPE_PARAM_REQUIRES_TYPE` | error | parameter must declare a type |
| `2-203-0038` | `TYPE_READ_ONLY_RECORD_UPDATE` | error | read-only record cannot be updated |
| `2-203-0039` | `TYPE_READ_ONLY_RECORD_CONSTRUCTOR` | error | read-only record cannot be constructed |
| `2-203-0040` | `TYPE_RESULT_IS_IMPLICIT` | error | Result return wrapping is implicit |
| `2-203-0041` | `TYPE_RETURN_MISMATCH` | error | return value type does not match function success type |
| `2-203-0042` | `TYPE_SUB_CANNOT_RETURN_VALUE` | error | SUB cannot return a value |
| `2-203-0043` | `TYPE_UNKNOWN_VALUE` | error | value type could not be determined |
| `2-203-0044` | `TYPE_UNKNOWN_ENUM_MEMBER` | error | enum member does not exist |
| `2-203-0045` | `TYPE_UNKNOWN_FIELD` | error | record field does not exist |
| `2-203-0046` | `TYPE_UNION_INCLUDE_REQUIRES_UNION` | error | union includes must name union types |
| `2-203-0048` | `TYPE_ASSIGN_REQUIRES_MUT` | error | assignment target must be mutable |
| `2-203-0049` | `TYPE_MATCH_PATTERN_MISMATCH` | error | match pattern type does not match the scrutinee type |
| `2-203-0050` | `TYPE_FOR_EACH_REQUIRES_COLLECTION` | error | FOR EACH source must be a List or Map |
| `2-203-0051` | `TYPE_LIST_ELEMENT_MISMATCH` | error | list element type does not match the expected element type |
| `2-203-0052` | `TYPE_MAP_KEY_MISMATCH` | error | map key type does not match the declared key type |
| `2-203-0053` | `TYPE_MAP_VALUE_MISMATCH` | error | map value type does not match the declared value type |
| `2-203-0055` | `TYPE_USE_AFTER_MOVE` | error | binding is used after move |
| `2-203-0056` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` | error | ordinary collections cannot store resource or thread ownership |
| `2-203-0100` | `TYPE_RESOURCE_ELEMENT_NOT_OWNER` | error | a collection element of resource type is a pointer, not an owner |
| `2-203-0101` | `TYPE_OVERLOAD_AMBIGUOUS` | error | return-type overload cannot be resolved without an expected type |
| `2-203-0102` | `TYPE_INSTANTIATION_TOO_DEEP` | error | template instantiation is too deep |
| `2-203-0058` | `TYPE_DUPLICATE_ARGUMENT_NAME` | error | call argument is supplied more than once |
| `2-203-0059` | `TYPE_UNKNOWN_ARGUMENT_NAME` | error | call argument name does not match any parameter |
| `2-203-0060` | `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` | error | uninitialized mutable binding requires a defaultable type |
| `2-203-0061` | `TYPE_REQUIRES_COMPARABLE` | error | operation requires a comparable type |
| `2-203-0062` | `TYPE_MATCH_NOT_EXHAUSTIVE` | error | match cases do not cover every possible value |
| `2-203-0063` | `TYPE_THREAD_NOT_SENDABLE` | error | thread boundary type is not sendable |
| `2-203-0064` | `TYPE_UNION_MEMBER_REQUIRES_TYPE` | error | union members must name concrete TYPE declarations |
| `2-203-0065` | `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` | error | recursive record cycle must pass through a List, Map, or UNION |
| `2-203-0066` | `TYPE_INLINE_TRAP_FALLS_THROUGH` | error | inline TRAP handler path neither recovers nor diverges |
| `2-203-0067` | `TYPE_RECOVER_TYPE_MISMATCH` | error | RECOVER value does not match the trapped expression's success type |
| `2-203-0068` | `TYPE_RECOVER_OUTSIDE_INLINE_TRAP` | error | RECOVER is valid only inside an inline TRAP handler |
| `2-203-0069` | `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` | error | inline TRAP requires a fallible call |
| `2-203-0104` | `TYPE_INLINE_TRAP_DEAD_HANDLER` | warn | inline TRAP handler is unreachable — the guarded call cannot fail |
| `2-203-0103` | `EXPORT_IN_EXECUTABLE` | error | EXPORT is only valid in a package project; use PUBLIC (the default) in an executable |
| `2-203-0105` | `TYPE_MONEY_LITERAL_OVERFLOW` | error | numeric literal is outside the Money range |
| `2-203-0106` | `TYPE_MONEY_LITERAL_UNDERFLOW` | error | numeric literal is outside the Money range |
| `2-203-0107` | `TYPE_MONEY_OPERATION_INVALID` | error | operation is not valid for Money operands |
| `2-203-0108` | `TYPE_MONEY_LITERAL_PRECISION` | warn | Money literal has more than 5 fractional digits and is rounded |
| `2-203-0109` | `MONEY_INEXACT_FLOAT_LITERAL` | warn | scaling Money by a bare decimal literal uses inexact Float arithmetic |
| `2-203-0110` | `TYPE_SCALAR_LITERAL_EMPTY` | error | a backtick scalar literal must contain exactly one Unicode scalar |
| `2-203-0111` | `TYPE_SCALAR_LITERAL_TOO_MANY` | error | a backtick scalar literal must contain exactly one Unicode scalar |
| `2-203-0112` | `TYPE_SCALAR_LITERAL_INVALID` | error | a scalar literal must name a valid Unicode scalar value |
| `2-203-0113` | `TYPE_ISOLATED_NOT_VISIBLE` | error | ISOLATED function must be a project-visible FUNC declaration |
| `2-203-0114` | `NATIVE_LIBRARY_MISSING` | error | a LINK block names a library with no matching project.json `libraries` entry |
| `2-203-0115` | `NATIVE_LIBRARY_TARGET_UNCOVERED` | warn | a supported build target has no `libraries` locator for a linked native library |
| `2-203-0116` | `NATIVE_LIBRARY_SOURCE_UNREADABLE` | error | a `vendor` locator's file under the project's `vendor/` directory is missing or cannot be read to hash it |
| `2-203-0117` | `NATIVE_LIBRARY_UNUSED` | warn | a project.json `libraries` entry has no matching LINK block in code |
| `2-203-0118` | `NATIVE_LIBRARY_NO_MATCH` | error | no native library locator matches the target being built |
| `2-203-0119` | `NATIVE_LIBRARY_AMBIGUOUS` | error | two equally-specific native library locators match the target being built |
| `2-203-0120` | `NATIVE_LIBRARY_FILE_MISSING` | error | a resolved `vendor` native library is missing from the consumer project's `vendor/` directory |
| `2-203-0121` | `NATIVE_LIBRARY_HASH_MISMATCH` | error | a resolved `vendor` native library does not match the sha256 the binding recorded for it |
| `2-203-0122` | `NATIVE_LIBRARY_VENDOR_COLLISION` | error | two declaring units vendor different native libraries that copy to the same output filename |
| `2-203-0123` | `NATIVE_ABI_UNKNOWN_CTYPE` | error | an ABI slot or return names a C type the marshaling backend does not implement |
| `2-203-0124` | `NATIVE_CSTRUCT_INVALID` | error | a CSTRUCT declaration is not a layout the compiler can compute faithfully |
| `2-203-0125` | `NATIVE_CSTRUCT_TOO_LARGE` | error | a CSTRUCT lays out larger than the maximum native struct size |
| `2-203-0126` | `NATIVE_CSTRUCT_ESCAPE` | error | a CSTRUCT name is used outside its LINK block, where only its mapped record type is nameable |
| `2-203-0127` | `NATIVE_STRUCT_FIELD_MISMATCH` | error | a CSTRUCT and the record it maps to differ by field name, type, or coverage |
| `2-203-0128` | `NATIVE_BIND_IN_INVALID` | error | a BIND IN block names an unknown slot or field, or binds a value it cannot marshal |
| `2-203-0070` | `TYPE_RESULT_NOT_USER_VISIBLE` | error | Result is an internal type and cannot be named in user code |
| `2-203-0071` | `TYPE_RESULT_NOT_MATCHABLE` | error | Ok and Error are not matchable as Result members in user code |
| `2-203-0072` | `TYPE_THREAD_RESULT_REMOVED` | error | the thread result field is removed; use thread::waitFor |
| `2-203-0073` | `SUB_RETURN_FORBIDDEN` | error | RETURN is forbidden in a SUB; use EXIT SUB |
| `2-203-0074` | `TYPE_SUB_HAS_NO_VALUE` | error | a SUB call produces no value and cannot be used in value position |
| `2-203-0075` | `EXIT_NO_MATCHING_LOOP` | error | EXIT has no matching enclosing loop |
| `2-203-0076` | `CONTINUE_NO_MATCHING_LOOP` | error | CONTINUE has no matching enclosing loop |
| `2-203-0077` | `EXIT_SUB_IN_FUNC` | error | EXIT SUB is valid only inside a SUB |
| `2-203-0078` | `EXIT_FUNC_FORBIDDEN` | error | EXIT FUNC is forbidden; functions must RETURN a value |
| `2-203-0079` | `TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` | error | EXIT PROGRAM requires an Integer exit code |
| `2-203-0080` | `EXIT_PROGRAM_CODE_OUT_OF_RANGE` | error | EXIT PROGRAM constant exit code is outside the host range |
| `2-203-0081` | `UNREACHABLE_AFTER_EXIT` | error | statement is unreachable after EXIT or CONTINUE |
| `2-203-0082` | `TYPE_RESOURCE_REQUIRES_RES` | error | resource must be bound with RES |
| `2-203-0083` | `TYPE_RES_REQUIRES_RESOURCE` | error | RES binds only resource types |
| `2-203-0084` | `TYPE_RESOURCE_FIELD_FORBIDDEN` | error | a record field cannot be a resource |
| `2-203-0085` | `TYPE_STATE_INVALID` | error | STATE must be a copyable, defaultable data type |
| `2-203-0086` | `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` | error | only the owning scope may close, return, or transfer a resource |
| `2-203-0087` | `TYPE_MIXED_RESOURCE_UNION` | error | a union must be all-data or all-resource, never mixed |
| `2-203-0088` | `TYPE_UNION_STATE_FORBIDDEN` | error | a resource union carries no STATE |
| `2-203-0129` | `TYPE_STATE_MISMATCH` | error | a resource's STATE type is fixed at its owning binding and every other declaration of it must agree |
| `2-203-0130` | `NATIVE_BIND_STATE_INVALID` | error | a BIND STATE must name the native function's stateful resource return and an OUT CSTRUCT slot whose record is the resource's STATE type |
| `2-203-0131` | `TYPE_RESOURCE_RETURN_ORDER` | error | a collection that carries a returned resource must be declared before that resource |
| `2-203-0132` | `NATIVE_BUFFER_INVALID` | error | a CBuffer slot or BUFFER SIZE clause is invalid: a CBuffer must be an OUT slot with exactly one BUFFER clause, named by RETURN, on a wrapper returning List OF Byte |
| `2-203-0089` | `RESOURCE_CLOSE_NOT_NATIVE` | error | a resource's CLOSE BY op must be a native LINK function |
| `2-203-0090` | `RESOURCE_CLOSE_MISSING` | error | a resource's CLOSE BY op names no function in its LINK block |
| `2-203-0091` | `RESOURCE_CLOSE_SIGNATURE` | error | a close op must consume exactly one RES parameter of its resource |
| `2-203-0092` | `NATIVE_CPTR_ESCAPE` | error | a raw C ABI type may appear only inside an ABI slot |
| `2-203-0093` | `NATIVE_ABI_RESULT_MARKER` | error | a native function's ABI result marker is malformed |
| `2-203-0094` | `NATIVE_ABI_UNBOUND_SLOT` | error | an ABI slot binds to no parameter, CONST pin, or result marker |
| `2-203-0095` | `NATIVE_ABI_UNBOUND_PARAM` | error | a native function parameter has no matching ABI slot |
| `2-203-0096` | `NATIVE_ABI_NO_RESULT` | error | a value-returning native function marks no ABI result |
| `2-203-0097` | `NATIVE_CONST_OUT` | error | a CONST-pinned ABI slot cannot also be OUT |
| `2-203-0098` | `NATIVE_CONST_UNKNOWN_SLOT` | error | a CONST pin names an unknown ABI slot |
| `2-203-0099` | `NATIVE_FREE_INVALID` | error | a FREE block is malformed: it must release the `return` CPtr produced slot through a deallocator taking one CPtr parameter and returning CVoid |

### `2-205` — Package metadata & DOC block semantics

This subsystem holds two independently-numbered blocks that **share** codes
`0001` and `0002` (see *Code Collisions*): a package-metadata pair, and the
resolver-stage DOC-semantics block (`0001`-`0021`).

Package metadata:

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-205-0001` | `PACKAGE_VERSION_UNSUPPORTED` | error | package binary representation or metadata version is unsupported |
| `2-205-0002` | `NATIVE_MANIFEST_INVALID` | error | native-link metadata in a package is malformed or inconsistent |

DOC block semantics (resolver):

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-205-0001` | `DOC_UNRESOLVED` | error | DOC header name does not resolve to any declaration in the package |
| `2-205-0002` | `DOC_NAME_MISMATCH` | error | DOC header keyword does not match the kind of the named declaration |
| `2-205-0003` | `DOC_DUPLICATE` | error | two DOC blocks name the same declaration |
| `2-205-0004` | `DOC_ARG_UNKNOWN` | error | ARG name does not match any parameter in the target signature |
| `2-205-0005` | `DOC_ARG_DUPLICATE` | error | two ARG lines document the same parameter |
| `2-205-0006` | `DOC_ARG_INVALID_CONTEXT` | error | ARG is not valid in a TYPE, UNION, ENUM, or PACKAGE doc block |
| `2-205-0007` | `DOC_PROP_UNKNOWN` | error | PROP name does not match any member of the target type |
| `2-205-0008` | `DOC_PROP_DUPLICATE` | error | two PROP lines document the same member |
| `2-205-0009` | `DOC_PROP_INVALID_CONTEXT` | error | PROP is valid only in TYPE, UNION, and ENUM doc blocks |
| `2-205-0010` | `DOC_RET_INVALID_CONTEXT` | error | RET is not valid in a TYPE, UNION, ENUM, or PACKAGE doc block |
| `2-205-0011` | `DOC_DUPLICATE_RET` | error | more than one RET line in a doc block |
| `2-205-0012` | `DOC_ERROR_INVALID_CONTEXT` | error | ERROR is not valid in a TYPE, UNION, ENUM, or PACKAGE doc block |
| `2-205-0013` | `DOC_DUPLICATE_EXAMPLE` | error | more than one EXAMPLE block in a doc block |
| `2-205-0014` | `DOC_DUPLICATE_PACKAGE` | error | more than one PACKAGE doc block in the package |
| `2-205-0015` | `DOC_DUPLICATE_ATTR` | error | the INTERNAL attribute appears more than once on one DOC line |
| `2-205-0016` | `DOC_UNKNOWN_ATTR` | error | unrecognized keyword in the DOC-line attribute position |
| `2-205-0017` | `DOC_INTERNAL_INVALID_CONTEXT` | error | INTERNAL is not valid on a PACKAGE doc block |
| `2-205-0018` | `DOC_DUPLICATE_DEPRECATED` | error | more than one DEPRECATED line in a doc block |
| `2-205-0019` | `DOC_GROUP_INVALID_CONTEXT` | error | GROUP is valid only on FUNC and SUB doc blocks |
| `2-205-0020` | `DOC_DUPLICATE_GROUP` | error | more than one GROUP line in a doc block |
| `2-205-0021` | `DOC_OVERLOAD_UNRESOLVED` | error | DOC header parameter types match no overload of the declaration |

### `2-208` — Test framework (assertion builtins)

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `2-208-0001` | `TESTING_EXPECT_OUTSIDE_TCASE` | error | assertion builtins are valid only inside a TCASE body |
| `2-208-0002` | `TESTING_EXPECT_ARITY` | error | assertion builtin called with the wrong number of arguments |
| `2-208-0003` | `TESTING_EXPECT_INCOMPARABLE` | error | expectEqual/expectNEqual operands must be comparable with `=` |
| `2-208-0004` | `TESTING_EXPECT_NOT_PRINTABLE` | error | expectEqual/expectNEqual operands must be printable for the failure message |
| `2-208-0005` | `TESTING_EXPECT_CODE_TYPE` | error | expectTrap expected-code argument must be an Integer |
| `2-208-0006` | `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` | error | expectTrap/expectNTrap require a call (not a package constant) to trap-guard |
| `2-208-0008` | `TESTING_EXPECT_TYPE_MISMATCH` | error | typed assertion operands must both be the named type |

### `3-302` — Verification

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `3-302-0001` | `VERIFICATION_FAILED` | error | binary representation or native validation failed |

### `3-304` — Target / codegen support

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `3-304-0001` | `TARGET_UNSUPPORTED` | error | requested target OS, CPU, or ABI is unsupported |
| `3-304-0002` | `PACKAGE_NATIVE_OUTPUT_UNSUPPORTED` | error | package projects do not support the requested native output mode |

### `5-500` — Linking

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `5-500-0001` | `LINK_FAILED` | error | linking packages, native libraries, symbols, objects, or executables failed |

### `6-603` — Lockfile

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `6-603-0001` | `LOCKFILE_MISMATCH` | error | resolved package state does not match mfb.lock |

### `6-605` — Package container / signing

Codes `0003`–`0007` are the client verification-chain refusals: each broken
link of pinned-server-key → attestation → pinned-ident → proof → one-off-key →
bytes gets its own code, emitted by the build gate after the
`uses <name> - [Tampered]` line.
[[src/cli/build.rs:classify_installed_package]]

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `6-605-0001` | `PACKAGE_INVALID` | error | package container is malformed or incompatible |
| `6-605-0002` | `PACKAGE_SIGNATURE_INVALID` | error | package signature, hash, or trust record is missing or invalid |
| `6-605-0003` | `PACKAGE_IDENT_KEY_UNTRUSTED` | error | package identKey does not match the pinned trust anchor |
| `6-605-0004` | `PACKAGE_ATTESTATION_INVALID` | error | package attestation is missing, unverifiable, or pins a different package |
| `6-605-0005` | `PACKAGE_PROOF_INVALID` | error | package proof is missing, unverifiable, or pins a different package |
| `6-605-0006` | `PACKAGE_PAYLOAD_HASH_MISMATCH` | error | package payload does not match the signed packageBinaryHash |
| `6-605-0007` | `PACKAGE_UNSIGNED_REMOTE` | error | unsigned package from a non-local source requires --unsigned |
| `6-605-0008` | `PACKAGE_IDENT_REANCHORED` | error | owner ident changed with no chain link from the pinned key; verify out-of-band |
| `6-605-0009` | `REGISTRY_LOG_ROLLBACK` | error | registry transparency log shrank or forked relative to the pinned checkpoint |
| `6-605-0010` | `PACKAGE_VENDOR_BLOB_MISSING` | error | registry has no blob for a vendored native library the package's section-10 table names |
| `6-605-0011` | `PACKAGE_VENDOR_BLOB_HASH_MISMATCH` | error | downloaded vendor blob does not match the sha256 recorded in the signed section-10 table |

### `0-000` — Fallback (synthetic)

Not a member of `RULES`; constructed inline by the lookup when a call site
names a rule that is not in the registry. [[src/rules/mod.rs:rule_for]]

| code | NAME | severity | message |
| --- | --- | --- | --- |
| `0-000-0000` | `UNKNOWN_RULE` | error | unknown diagnostic rule |

## See Also

* ./mfb spec language error-model — how runtime errors and fallible calls surface at the source level
* ./mfb spec language types — the type, ownership, and resource rules the `2-203` checks enforce
* ./mfb spec architecture commands — the build pipeline whose phases the group `G` axis mirrors
* ./mfb spec memory fallible-call-abi — the success/error register contract underlying `2-200-0100` build failures
