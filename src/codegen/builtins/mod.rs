//! Builtin packages whose lowering has migrated into the codegen layer
//! (plan-95). Each package owns its `BuiltinFunction` descriptors and, per
//! migrated function, the target-generic lowering carried by `Implementation`.

pub(crate) mod collections;
pub(crate) mod csv;
pub(crate) mod encoding;
pub(crate) mod json;
