//! Builtin packages whose lowering has migrated into the codegen layer
//! (plan-95). Each package owns its `BuiltinFunction` descriptors and, per
//! migrated function, the target-generic lowering carried by `Implementation`.

pub(crate) mod bits;
pub(crate) mod collections;
pub(crate) mod crypto;
pub(crate) mod csv;
pub(crate) mod datetime;
pub(crate) mod encoding;
pub(crate) mod errorcode;
pub(crate) mod fs;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod math;
pub(crate) mod money;
pub(crate) mod os;
pub(crate) mod process;
pub(crate) mod regex;
pub(crate) mod thread;
pub(crate) mod tls;
