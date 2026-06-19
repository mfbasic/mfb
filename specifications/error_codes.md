# MFBASIC Error Codes

This document is the canonical error-code registry for MFBASIC.

- Canonical code format: `G-SSS-EEEE`
- `G` = generator
- `SSS` = subsystem
- `EEEE` = concrete error within that subsystem
- Runtime `Error.code` integer values are the canonical code with hyphens removed. For example, `7-705-0002` is stored as `77050002`.
- Compiler and toolchain diagnostics print the hyphenated code together with the symbolic rule or error name.

## Generators and Subsystems

| Code      | Meaning                                                          |
|-----------|------------------------------------------------------------------|
| `1-100-*` | Parser / source loading                                          |
| `1-101-*` | Parser / lexer                                                   |
| `1-102-*` | Parser / AST construction and syntax diagnostics                 |
| `2-200-*` | Compiler front / manifest, build orchestration, entrypoint       |
| `2-201-*` | Compiler front / name resolution                                 |
| `2-203-*` | Compiler front / type checking and static semantics              |
| `2-205-*` | Compiler front / bytecode and package ABI                        |
| `3-302-*` | Compiler back / native validation                                |
| `3-304-*` | Compiler back / target-specific planning                         |
| `5-500-*` | Linker / host object-plan and final link                         |
| `6-603-*` | Package manager / dependency and lock resolution                 |
| `6-605-*` | Package manager / package verification                           |
| `7-701-*` | Runtime / memory and allocation                                  |
| `7-702-*` | Runtime / I/O                                                    |
| `7-703-*` | Runtime / filesystem and resource handles                        |
| `7-705-*` | Runtime / package helpers, builtins, and generic language errors |
| `7-706-*` | Runtime / trap and failure propagation                           |
| `7-707-*` | Runtime / platform ABI and networking                            |

## Runtime and Standard Package Errors

These are the canonical `errorCode::*` values and the integer payloads used by `Error.code`.

| Code         | Integer    | Name                  | Meaning | Notes |
|--------------|------------|-----------------------|---------|-------|
| `7-705-0000` | `77050000` | `ErrUnknown`          | Unclassified standard-package failure. | |
| `7-705-0001` | `77050001` | `ErrIndexOutOfRange`  | List or string index/range is outside valid bounds. | |
| `7-705-0002` | `77050002` | `ErrInvalidArgument`  | Argument value is not valid for the requested operation. | |
| `7-705-0003` | `77050003` | `ErrInvalidFormat`    | Text parse or non-finite numeric representation conversion failed. | |
| `7-705-0004` | `77050004` | `ErrNotFound`         | Requested item, key, file, or resource was not found. | |
| `7-705-0005` | `77050005` | `ErrAlreadyExists`    | Create operation conflicts with an existing item. | |
| `7-705-0006` | `77050006` | `ErrPermissionDenied` | Operation is not permitted by the host environment. | |
| `7-705-0007` | `77050007` | `ErrUnsupported`      | Operation is not supported by the implementation or platform. | |
| `7-705-0008` | `77050008` | `ErrTimeout`          | Operation did not complete before its deadline. | |
| `7-705-0009` | `77050009` | `ErrInterrupted`      | Operation was interrupted before completion. | |
| `7-701-0001` | `77010001` | `ErrOutOfMemory`      | Allocation failed. | |
| `7-703-0001` | `77030001` | `ErrPathNotFound`     | Filesystem path does not exist. | |
| `7-703-0002` | `77030002` | `ErrInvalidPath`      | Filesystem path string is invalid for the host platform. | |
| `7-703-0003` | `77030003` | `ErrAccessDenied`     | Filesystem access was denied. | |
| `7-702-0001` | `77020001` | `ErrReadFailed`       | Read operation failed. | |
| `7-702-0002` | `77020002` | `ErrWriteFailed`      | Write or flush operation failed. | Internal docs previously called this `ErrWriteFailed`. |
| `7-702-0003` | `77020003` | `ErrEndOfFile`        | Read operation reached end of file where a value was required. | |
| `7-703-0004` | `77030004` | `ErrResourceClosed`   | Resource handle is already closed. | |
| `7-703-0005` | `77030005` | `ErrResourceBusy`     | Resource is unavailable, locked, busy, or not in the required empty state. | Delete-directory docs previously called this `ErrResourceBusy`. |
| `7-702-0004` | `77020004` | `ErrEncoding`         | Text encoding or decoding failed. | |
| `7-702-0005` | `77020005` | `ErrInputFailed`      | Standard input operation failed. | |
| `7-707-0001` | `77070001` | `ErrAddressInvalid`   | Network host, address, or port is invalid. | |
| `7-707-0002` | `77070002` | `ErrAddressNotFound`  | Network host name or address could not be resolved. | |
| `7-707-0003` | `77070003` | `ErrNetworkFailed`    | Network operation failed before a connection was established. | |
| `7-707-0004` | `77070004` | `ErrConnectionClosed` | Socket peer closed the connection or the connection is no longer usable. | |
| `7-707-0005` | `77070005` | `ErrReadTimeout`      | Socket read operation timed out. | |
| `7-707-0006` | `77070006` | `ErrWriteTimeout`     | Socket write operation timed out. | |
| `7-707-0007` | `77070007` | `ErrMessageTooLarge`  | Datagram or message exceeds the requested or supported size. | |
| `7-705-0010` | `77050010` | `ErrOverflow`         | Arithmetic overflow or numeric conversion outside the destination range. | |
| `7-703-0006` | `77030006` | `ErrCloseFailed`      | Resource close operation failed. | |
| `7-707-0008` | `77070008` | `ErrTlsFailed`        | TLS handshake, certificate validation, SNI validation, or protocol operation failed. | |
| `7-705-0011` | `77050011` | `ErrUnderflow`        | Arithmetic underflow below the destination range. | |
| `7-705-0012` | `77050012` | `ErrFloatDomain`      | Floating-point operation domain is invalid, including divide-by-zero. | |
| `7-705-0013` | `77050013` | `ErrFloatNaN`         | Floating-point operation produced a NaN result. | |
| `7-705-0014` | `77050014` | `ErrFloatInf`         | Floating-point operation produced an infinity result. | |
| `7-705-0015` | `77050015` | `ErrFloatOverflow`    | Floating-point arithmetic overflowed to infinity. | |
| `7-706-0001` | `77060001` | `ErrWrapped`          | Generic wrapper code for adding context while preserving the underlying message. | |

## Compiler Diagnostics

These codes correspond to `src/rules.rs` and are emitted directly by the compiler.

### Manifest, Build, and Entrypoint

| Code         | Rule                          | Severity | Message |
|--------------|-------------------------------|----------|---------|
| `2-200-0001` | `PROJECT_JSON_MISSING`        | error    | `project.json is required` |
| `2-200-0002` | `PROJECT_JSON_READ_FAILED`    | error    | `project.json could not be read` |
| `2-200-0003` | `PROJECT_JSON_PARSE_FAILED`   | error    | `project.json is not valid JSON` |
| `2-200-0004` | `PROJECT_JSON_ROOT_TYPE`      | error    | `project.json must contain a JSON object` |
| `2-200-0005` | `PROJECT_JSON_REQUIRED_FIELD` | error    | `project.json is missing a required field` |
| `2-200-0006` | `PROJECT_JSON_FIELD_TYPE`     | error    | `project.json field has the wrong type` |
| `2-200-0007` | `PROJECT_JSON_EMPTY_FIELD`    | error    | `project.json field must not be empty` |
| `2-200-0008` | `PROJECT_JSON_EMPTY_SOURCES`  | error    | `project.json must include at least one source entry` |
| `2-200-0009` | `PROJECT_JSON_UNKNOWN_KIND`   | warn     | `project.json kind is not recognized` |
| `2-200-0010` | `PROJECT_JSON_VALID`          | info     | `project.json passed validation` |
| `2-200-0011` | `PROJECT_ENTRY_INVALID`       | error    | `project entry point is invalid` |

### Source Loading and Parsing

| Code | Rule | Severity | Message |
|------|------|----------|---------|
| `1-100-0001` | `MFB_SOURCE_READ_FAILED` | error | `MFBASIC source could not be read` |
| `1-100-0002` | `MFB_SOURCE_ROOT_MISSING` | error | `MFBASIC source root does not exist` |
| `1-100-0003` | `MFB_SOURCE_EMPTY` | error | `MFBASIC source root contains no source files` |
| `1-100-0004` | `MFB_SOURCE_OUTSIDE_PROJECT` | error | `MFBASIC source path resolves outside the project directory` |
| `1-100-0005` | `MFB_SOURCE_OVERLAP` | error | `MFBASIC source file is selected by more than one source entry` |
| `1-101-0001` | `MFB_LEX_UNEXPECTED_CHARACTER` | error | `lexer found an unexpected character` |
| `1-101-0002` | `MFB_LEX_UNTERMINATED_STRING` | error | `string literal is unterminated` |
| `1-102-0001` | `MFB_PARSE_EXPECTED_EXPRESSION` | error | `parser expected an expression` |
| `1-102-0002` | `MFB_PARSE_INVALID_FUNCTION_HEADER` | error | `function header is invalid` |
| `1-102-0003` | `MFB_PARSE_INVALID_IDENTIFIER` | error | `identifier is invalid` |
| `1-102-0004` | `MFB_PARSE_UNEXPECTED_STATEMENT` | error | `parser found an unexpected statement` |
| `1-102-0005` | `MFB_PARSE_UNEXPECTED_TOKEN` | error | `parser found an unexpected token` |
| `1-102-0006` | `MFB_PARSE_UNTERMINATED_BLOCK` | error | `parser reached end-of-file inside a block` |
| `1-102-0007` | `MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING` | error | `pipeline expression is missing a placeholder` |

### Name Resolution

| Code | Rule | Severity | Message |
|------|------|----------|---------|
| `2-201-0001` | `IMPORT_MISSING_PACKAGE` | error | `imported package could not be resolved` |
| `2-201-0002` | `IMPORT_PACKAGE_NOT_DECLARED` | error | `imported package is not declared` |
| `2-201-0003` | `IMPORT_PACKAGE_NOT_INSTALLED` | error | `declared package is not installed` |
| `2-201-0004` | `IMPORT_LOCAL_PATH_INVALID` | error | `local package source must be an absolute local URL` |
| `2-201-0005` | `IMPORT_PACKAGE_MANIFEST_INVALID` | error | `imported package manifest is invalid` |
| `2-201-0006` | `IMPORT_PACKAGE_NAME_MISMATCH` | error | `imported package manifest name does not match import` |
| `2-201-0007` | `IMPORT_PACKAGE_KIND_INVALID` | error | `imported source package must be a package` |
| `2-201-0008` | `SYMBOL_DUPLICATE_IMPORT` | error | `import is declared more than once` |
| `2-201-0009` | `SYMBOL_DUPLICATE_LOCAL` | error | `local symbol is declared more than once` |
| `2-201-0010` | `SYMBOL_DUPLICATE_TOP_LEVEL` | error | `top-level symbol is declared more than once` |
| `2-201-0011` | `SYMBOL_UNKNOWN_IDENTIFIER` | error | `identifier could not be resolved` |
| `2-201-0012` | `SYMBOL_NOT_CALLABLE` | error | `symbol cannot be called` |
| `2-201-0013` | `SYMBOL_NOT_VALUE` | error | `symbol is not a value` |
| `2-201-0014` | `SYMBOL_UNKNOWN_IMPORT` | error | `package-qualified symbol uses an unknown import` |
| `2-201-0015` | `SYMBOL_UNKNOWN_TYPE` | error | `type name could not be resolved` |

### Type Checking and Static Semantics

| Code | Rule | Severity | Message |
|------|------|----------|---------|
| `2-203-0001` | `TYPE_BINARY_OPERATOR_MISMATCH` | error | `binary operator operands have incompatible types` |
| `2-203-0002` | `TYPE_UNARY_OPERATOR_MISMATCH` | error | `unary operator operand has an incompatible type` |
| `2-203-0003` | `TYPE_UNARY_OPERATOR_UNKNOWN` | error | `unary operator is not recognized` |
| `2-203-0004` | `TYPE_FOR_REQUIRES_NUMERIC` | error | `FOR loop operands must be numeric` |
| `2-203-0005` | `TYPE_FOR_STEP_ZERO` | error | `FOR loop step must not be zero` |
| `2-203-0006` | `TYPE_CONDITION_REQUIRES_BOOLEAN` | error | `control-flow condition must be Boolean` |
| `2-203-0007` | `TYPE_BINDING_MISMATCH` | error | `binding initializer type does not match declared type` |
| `2-203-0008` | `TYPE_ASSIGNMENT_MISMATCH` | error | `assignment value type does not match binding type` |
| `2-203-0009` | `TYPE_INTEGER_LITERAL_OVERFLOW` | error | `integer literal is outside the Integer range` |
| `2-203-0010` | `TYPE_FAIL_REQUIRES_ERROR` | error | `FAIL requires an Error value` |
| `2-203-0011` | `TYPE_PROPAGATE_REQUIRES_TRAP` | error | `PROPAGATE requires a TRAP context` |
| `2-203-0012` | `TYPE_TRAP_FALLTHROUGH` | error | `TRAP path can fall through` |
| `2-203-0013` | `TYPE_BYTE_LITERAL_OVERFLOW` | error | `integer literal is outside the Byte range` |
| `2-203-0014` | `TYPE_BYTE_LITERAL_UNDERFLOW` | error | `integer literal is outside the Byte range` |
| `2-203-0015` | `TYPE_FLOAT_LITERAL_OVERFLOW` | error | `numeric literal is outside the Float range` |
| `2-203-0016` | `TYPE_FLOAT_LITERAL_UNDERFLOW` | error | `numeric literal is outside the Float range` |
| `2-203-0017` | `TYPE_FIXED_LITERAL_OVERFLOW` | error | `numeric literal is outside the Fixed range` |
| `2-203-0018` | `TYPE_FIXED_LITERAL_UNDERFLOW` | error | `numeric literal is outside the Fixed range` |
| `2-203-0019` | `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` | error | `lambda capture is invalid` |
| `2-203-0020` | `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE` | error | `binding requires a type annotation or initializer` |
| `2-203-0021` | `TYPE_CALL_ARGUMENT_MISMATCH` | error | `function call argument type does not match parameter type` |
| `2-203-0022` | `TYPE_CALL_ARITY_MISMATCH` | error | `function call has the wrong number of arguments` |
| `2-203-0023` | `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` | error | `constructor argument type does not match field type` |
| `2-203-0024` | `TYPE_CONSTRUCTOR_ARITY_MISMATCH` | error | `constructor has the wrong number of arguments` |
| `2-203-0025` | `TYPE_CONSTRUCTOR_REQUIRES_RECORD` | error | `record constructor syntax requires a TYPE` |
| `2-203-0026` | `TYPE_DEFAULT_ARG_ORDER` | error | `default parameters must be trailing` |
| `2-203-0027` | `TYPE_DEFAULT_VALUE_MISMATCH` | error | `default parameter value has the wrong type` |
| `2-203-0028` | `TYPE_DUPLICATE_ENUM_MEMBER` | error | `enum member is declared more than once` |
| `2-203-0029` | `TYPE_DUPLICATE_FIELD` | error | `type field is declared more than once` |
| `2-203-0030` | `TYPE_DUPLICATE_VARIANT` | error | `union member type is declared more than once` |
| `2-203-0031` | `TYPE_ENUM_REQUIRES_MEMBER` | error | `enum must declare at least one member` |
| `2-203-0032` | `TYPE_FUNC_MISSING_RETURN` | error | `function is missing a return value` |
| `2-203-0033` | `TYPE_FUNC_REQUIRES_RETURN_TYPE` | error | `FUNC must declare a return type` |
| `2-203-0034` | `TYPE_FIELD_ACCESS_REQUIRES_RECORD` | error | `field access requires a record value` |
| `2-203-0035` | `TYPE_LET_REQUIRES_VALUE` | error | `immutable binding must have an initializer` |
| `2-203-0036` | `TYPE_MEMBER_NOT_VISIBLE` | error | `type member is not visible from this scope` |
| `2-203-0037` | `TYPE_PARAM_REQUIRES_TYPE` | error | `parameter must declare a type` |
| `2-203-0038` | `TYPE_READ_ONLY_RECORD_UPDATE` | error | `read-only record cannot be updated` |
| `2-203-0039` | `TYPE_READ_ONLY_RECORD_CONSTRUCTOR` | error | `read-only record cannot be constructed` |
| `2-203-0040` | `TYPE_RESULT_IS_IMPLICIT` | error | `Result return wrapping is implicit` |
| `2-203-0041` | `TYPE_RETURN_MISMATCH` | error | `return value type does not match function success type` |
| `2-203-0042` | `TYPE_SUB_CANNOT_RETURN_VALUE` | error | `SUB cannot return a value` |
| `2-203-0043` | `TYPE_UNKNOWN_VALUE` | error | `value type could not be determined` |
| `2-203-0044` | `TYPE_UNKNOWN_ENUM_MEMBER` | error | `enum member does not exist` |
| `2-203-0045` | `TYPE_UNKNOWN_FIELD` | error | `record field does not exist` |
| `2-203-0046` | `TYPE_UNION_INCLUDE_REQUIRES_UNION` | error | `union includes must name union types` |
| `2-203-0047` | `TYPE_VARIANT_CONSTRUCTOR_AMBIGUOUS` | error | `union member constructor name is ambiguous` |
| `2-203-0048` | `TYPE_ASSIGN_REQUIRES_MUT` | error | `assignment target must be mutable` |
| `2-203-0049` | `TYPE_MATCH_PATTERN_MISMATCH` | error | `match pattern type does not match the scrutinee type` |
| `2-203-0050` | `TYPE_FOR_EACH_REQUIRES_COLLECTION` | error | `FOR EACH source must be a List or Map` |
| `2-203-0051` | `TYPE_LIST_ELEMENT_MISMATCH` | error | `list element type does not match the expected element type` |
| `2-203-0052` | `TYPE_MAP_KEY_MISMATCH` | error | `map key type does not match the declared key type` |
| `2-203-0053` | `TYPE_MAP_VALUE_MISMATCH` | error | `map value type does not match the declared value type` |
| `2-203-0054` | `TYPE_USING_REQUIRES_RESOURCE` | retired | `retired: USING removed from the language; resources close by lexical drop` |
| `2-203-0055` | `TYPE_USE_AFTER_MOVE` | error | `binding is used after move` |
| `2-203-0056` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` | error | `ordinary collections cannot store resource or thread ownership` |
| `2-203-0057` | `TYPE_DOUBLE_DROP_PATH` | error | `control flow could drop the same owned value more than once` |
| `2-203-0058` | `TYPE_DUPLICATE_ARGUMENT_NAME` | error | `call argument is supplied more than once` |
| `2-203-0059` | `TYPE_UNKNOWN_ARGUMENT_NAME` | error | `call argument name does not match any parameter` |
| `2-203-0060` | `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` | error | `uninitialized mutable binding requires a defaultable type` |
| `2-203-0061` | `TYPE_REQUIRES_COMPARABLE` | error | `operation requires a comparable type` |
| `2-203-0062` | `TYPE_MATCH_NOT_EXHAUSTIVE` | error | `match cases do not cover every possible value` |
| `2-203-0063` | `TYPE_THREAD_NOT_SENDABLE` | error | `thread boundary type is not sendable` |
| `2-203-0064` | `TYPE_UNION_MEMBER_REQUIRES_TYPE` | error | `union members must name concrete TYPE declarations` |
| `2-203-0065` | `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` | error | `recursive record cycle must pass through a List, Map, or UNION` |
| `2-203-0066` | `TYPE_INLINE_TRAP_FALLS_THROUGH` | error | `inline TRAP handler path neither recovers nor diverges` |
| `2-203-0067` | `TYPE_RECOVER_TYPE_MISMATCH` | error | `RECOVER value does not match the trapped expression's success type` |
| `2-203-0068` | `TYPE_RECOVER_OUTSIDE_INLINE_TRAP` | error | `RECOVER is valid only inside an inline TRAP handler` |
| `2-203-0069` | `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` | error | `inline TRAP requires a fallible call` |

## Toolchain and Package Diagnostics

These are non-runtime diagnostics emitted by the `mfb` application and therefore belong in `src/rules.rs` like the rest of the compiler-facing rule set.

| Code         | Rule                            | Severity | Message |
|--------------|---------------------------------|----------|---------|
| `2-200-0100` | `BUILD_FAILED`                  | error    | `build failed for an unclassified orchestration reason` |
| `2-205-0001` | `PACKAGE_VERSION_UNSUPPORTED`   | error    | `package bytecode or metadata version is unsupported` |
| `2-205-0002` | `NATIVE_MANIFEST_INVALID`       | error    | `native-link metadata in a package is malformed or inconsistent` |
| `3-302-0001` | `VERIFICATION_FAILED`           | error    | `bytecode or native validation failed` |
| `3-304-0001` | `TARGET_UNSUPPORTED`            | error    | `requested target OS, CPU, or ABI is unsupported` |
| `3-304-0002` | `PACKAGE_NATIVE_OUTPUT_UNSUPPORTED` | error | `package projects do not support the requested native output mode` |
| `5-500-0001` | `LINK_FAILED`                   | error    | `linking packages, native libraries, symbols, objects, or executables failed` |
| `6-603-0001` | `LOCKFILE_MISMATCH`             | error    | `resolved package state does not match mfb.lock` |
| `6-605-0001` | `PACKAGE_INVALID`               | error    | `package container is malformed or incompatible` |
| `6-605-0002` | `PACKAGE_SIGNATURE_INVALID`     | error    | `package signature, hash, or trust record is missing or invalid` |

## Notes

- This registry supersedes the older flat `10000`/`20000`/`30000` documentation bands.
- Older umbrella compiler names such as `ErrSyntax` are intentionally replaced here by the concrete emitted rule identifiers from `src/rules.rs`.
- Non-runtime diagnostics emitted by the `mfb` process should be added to `src/rules.rs` and documented here in `Code | Rule | Severity | Message` form.
- Runtime `Error.code` values remain the separate exported constant registry because they are program-visible data, not host-tool diagnostics.
