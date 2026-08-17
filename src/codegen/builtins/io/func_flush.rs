//! `io::flush` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`super::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally). Docs migrated from
//! `src/docs/man/builtins/io/flush.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Drain the per-thread standard-output buffer"#;
const DESC: &str = r#"`io::flush` writes out any bytes currently held in this thread's MFBASIC
standard-output buffer and returns nothing. It takes no arguments.

The call is **drain-only**. It issues the pending bytes with a `write` loop and
reports whether that write succeeded; it deliberately does *not* `fsync` or
otherwise ask the host to sync standard output. The buffer drain's `write` is the
one portable failure signal, identical on every platform and libc.

It follows that `io::flush` is a **no-op when buffering is off** — the default.
Without `io::setBuffered(TRUE)` there is no MFBASIC buffer to drain, every
`io::write` and `io::print` has already reached the operating system, and this
call succeeds having done nothing. It is likewise a no-op when buffering is on
but nothing is pending.

The drain loops until the buffer is empty: a short write advances the cursor and
re-issues, and an `EINTR` interruption retries. If a write genuinely fails, the
still-unflushed bytes are slid back to the base of the buffer and kept, so a later
`io::flush` resumes from exactly where this one stopped — and this call raises
`ErrOutput`.

An explicit flush is rarely required even under buffering: the buffer is also
drained when it fills, before every standard-input read, on
`io::setBuffered(FALSE)`, and at program exit. Standard error is never buffered
and is written immediately, so it has no corresponding flush. In app mode
transcript writes are synchronous, so this call succeeds immediately."#;
const EX: &str = r#"Make buffered output visible at a checkpoint:

```
IMPORT io

SUB longRunningWork()
END SUB

SUB main()
  io::setBuffered(TRUE)
  io::print("phase one complete")
  io::flush()                ' the line reaches the terminal before the long work
  longRunningWork()
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "flush",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::lower_io_helper),
                Some(super::lower_io_helper),
                &[],
            ),
        }],
    });
}
