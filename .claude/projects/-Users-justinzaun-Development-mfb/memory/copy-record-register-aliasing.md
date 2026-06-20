---
name: copy-record-register-aliasing
description: Latent aliasing bug in thread-transfer copy loops where copied field value can be clobbered by the x9 result-pointer reload
metadata:
  type: reference
---

The thread-transfer field-copy loops in `src/target/shared/code/builder_misc.rs`
(`copy_record_to_current_arena`, `copy_record_fields_into_existing`,
`copy_union_to_current_arena`, `copy_error_to_current_arena`, and the
collection-payload copy) had a latent register-aliasing bug:

```rust
let copied = self.copy_value_to_current_arena(field_type, "x10")?; // copied is an allocate_register() reg
self.emit(abi::load_u64("x9", sp, result_slot));   // if copied == x9, this clobbers it!
self.emit(abi::store_u64(&copied, "x9", offset));   // stores the result pointer, not the value
```

`copy_value_to_current_arena` for a scalar field returns a register from the
allocate-register pool (x8–x17/x20–x28). When register pressure pushes that
allocation onto `x9`, the subsequent `load x9 <- result_slot` clobbers the copied
value and the store writes the record's own pointer into the field — a
non-deterministic ASLR pointer at runtime (looks like garbage, varies per run).

Symptom: a record/union returned across a thread boundary has a correct first
field but a garbage later field. The bug is layout/pressure-sensitive, so adding
unrelated code (which shifts `next_register`) makes it appear/disappear — like the
macOS GOT bug in [[macos-codegen-latent-bugs]].

Fix: stash `copied` to a stack slot before reloading the result pointer:
`store copied -> field_slot; load x9 <- result_slot; load x10 <- field_slot; store x10 -> [x9, off]`.

It was exposed by the read-only-`Error` work (the `error(...)` source-location
ErrorLoc allocations raised register pressure in surrounding code).
