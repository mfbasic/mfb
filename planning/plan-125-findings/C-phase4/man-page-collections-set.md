### 1. Map key parameter described as a list index
UNIT:      man-page:collections/set
CLAIM:     "The list index to overwrite, zero-based. It must already exist; set does not extend a list."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_set.rs:122-147` attaches this text to the `Map OF K TO V` overload. The scratch map example compiled and printed `20` after inserting then overwriting `"Ada"`, proving a missing map key is inserted rather than rejected.
SUGGESTED: "The map key to assign. A missing key is inserted; no numeric range applies."

### 2. Errors table falsely attributes list bounds error to map overload
UNIT:      man-page:collections/set
CLAIM:     "ErrIndexOutOfRange … Overloads 1, 2"
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_set.rs:119,147` registers `ErrIndexOutOfRange` for both overloads, but `lower_set`’s map branch (`func_set.rs:268-347`) routes through `lower_map_remove_key` and map concatenation, with no index bounds check. The map probe inserted absent key `"Ada"` and printed `20`; it did not raise.
SUGGESTED: Show `ErrIndexOutOfRange` for overload 1 only.

### 3. Assignment-shape implementation detail is out of scope
UNIT:      man-page:collections/set
CLAIM:     "Assigning straight back to the same local variable — c = collections::set(c, k, v) — is the cheap shape: it updates the collection in place rather than rebuilding it."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3 bans lowering/code-generation mechanics from developer pages. `src/codegen/builtins/collections/func_set.rs:168-347` and `src/codegen/collection/assign/builder_inplace_assign.rs` show this is context-dependent compiler behavior, not a language-visible contract.
SUGGESTED: "Assign the returned collection to retain the update: `c = collections::set(c, k, v)`."

### 4. Copying/quadratic claim teaches a compiler memory model
UNIT:      man-page:collections/set
CLAIM:     "Writing through something reached another way (a parameter, a module-level MUT, or the collection a FOR EACH is walking) copies the whole collection on every write, which turns a write-heavy loop quadratic."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §§3–4 prohibit compiler mechanics and explanations that teach an ownership-style mental model without banned words. The claim depends on assignment-context optimization choices in `src/codegen/collection/assign/builder_inplace_assign.rs`, not observable `collections::set` semantics.
SUGGESTED: Delete this sentence.