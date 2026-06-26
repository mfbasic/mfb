# MFBASIC Threading

This document specifies how MFBASIC threads are compiled, linked, and executed.
The authoritative source-level API reference is the built-in `thread` package
(`./mfb man thread`); this document describes the implementation contract behind
it. See the See Also section for the related specs.

Last updated: 2026-06-26

The source-level API is the built-in `thread` package. This document describes
the implementation contract behind that API.

## Goals

MFBASIC threads provide isolated package workers with typed input, output, and
two communication channels: a **data plane** (`thread::send`/`thread::receive`/
`thread::poll`) carrying copyable values, and a **resource plane**
(`thread::transfer`/`thread::accept`) carrying move-only resource handles.

The model has these requirements:

- A thread entry point is an exported `ISOLATED FUNC` from an imported package.
- The worker runs in a native OS thread.
- The worker receives its own thread handle and one input value.
- The parent communicates with the worker through bounded typed queues, split by
  direction (inbound parent→worker, outbound worker→parent) on each plane.
- All values that cross a thread boundary have thread-sendable types.
- The data plane is resource-free; sendable resource handles move on the
  dedicated resource plane without being copied.
- The worker outcome is stored internally and retrieved exactly once by `thread::waitFor(t)`, closing the parent `Thread` handle.
- Package imports used by the worker must work inside the worker thread exactly
  as they work outside a thread.
- Native code generation must resolve all worker and package calls at link time;
  no dynamic source-level package lookup occurs at runtime.

## Reading order

The topics below follow the worker lifecycle from source to execution.
`source-model` defines the thread entry shape, the `Thread`/`ThreadWorker` type
grammar (including the optional `RES Res` resource plane), and the
thread-sendability rules; `isolation` specifies what an `ISOLATED` worker may
touch and how boundary values are transferred. `package-requirements`,
`function-ids-and-package-calls`, and `worker-and-package-functions` cover how
thread workers live in `.mfp` packages, how calls reference functions across
packages, and how every merged package function lowers through the single
`IR -> NIR -> native` codegen path. `thread-runtime-helpers`, `control-block`,
and `queue-semantics` specify the runtime contract: the full helper-symbol set,
the native control-block and queue-record layouts, the direction-split internal
lowering of the data and resource planes, and the bounded-queue, cancellation,
and cleanup behavior. `os-integration` and `linking-requirements` give the
concrete macOS/Linux thread creation and the native linking obligations;
`error-propagation` covers how worker results (and their source locations) flow
back through `thread::waitFor`; and `validation` lists the runtime test coverage
a complete implementation must include.

## See Also

* ./mfb spec language threads — the source-level thread model
* ./mfb spec architecture — the build pipeline that produces workers
* ./mfb spec memory — per-arena isolation and flat-value copies
* ./mfb spec package — worker packages carried in `.mfp` files
* ./mfb spec linker — thread linking requirements
* ./mfb man thread — the source-level thread API
