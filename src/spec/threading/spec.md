# MFBASIC Threading

This document specifies how MFBASIC threads are compiled, linked, and executed.
It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/memory_layouts.md`

Last updated: 2026-06-17

The source-level API is the built-in `thread` package. This document describes
the implementation contract behind that API.

## Goals

MFBASIC threads provide isolated package workers with typed input, output, and
message channels.

The model has these requirements:

- A thread entry point is an exported `ISOLATED FUNC` from an imported package.
- The worker runs in a native OS thread.
- The worker receives its own thread handle and one input value.
- The parent communicates with the worker through bounded typed queues.
- All values that cross a thread boundary have thread-sendable types.
- Sendable resource handles move across thread queues without being copied.
- The worker outcome is stored internally and retrieved exactly once by `thread::waitFor(t)`, closing the parent `Thread` handle.
- Package imports used by the worker must work inside the worker thread exactly
  as they work outside a thread.
- Native code generation must resolve all worker and package calls at link time;
  no dynamic source-level package lookup occurs at runtime.

## Reading order

The topics below follow the worker lifecycle from source to execution.
`source-model` defines the thread entry shape and thread-sendability rules;
`isolation` specifies what an `ISOLATED` worker may touch and how boundary values
are transferred. `package-requirements`, `function-ids-and-package-calls`, and
`worker-and-package-functions` cover how thread workers live in `.mfp` packages,
how calls reference functions across packages, and how every merged package
function lowers through the single `IR -> NIR -> native` codegen path.
`thread-runtime-helpers`, `control-block`, and `queue-semantics` specify the
runtime contract: helper symbols, the native control-block layout, and the
bounded-queue, cancellation, and cleanup behavior. `os-integration` and
`linking-requirements` give the concrete macOS/Linux thread creation and the
native linking obligations; `error-propagation` covers how worker results flow
back through `thread::waitFor`; and `validation` lists the runtime test coverage
a complete implementation must include.
