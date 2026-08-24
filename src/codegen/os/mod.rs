//! Shared OS-seam (runtime-call) primitives.
//!
//! An OS-seam member lowers to a `_mfb_rt_<pkg>_*` runtime helper whose body is
//! **arch-neutral** `abi::` code that branches only on OS family (libc vs
//! kernel32). Each member owns that emission in its `func_*.rs` via its
//! `Body::abi_function` body on the clean-room registry, wrapped once by
//! `lower_abi_function_helper`. This module holds the shared FFI/syscall/process
//! primitives those bodies build on.

pub(crate) mod ffi;
pub(crate) mod process;
pub(crate) mod syscall;
