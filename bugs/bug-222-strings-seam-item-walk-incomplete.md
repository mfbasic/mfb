# bug-222: scalar-seam usage walker skips non-Function/Binding items (latent companion-injection miss)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness (latent)

Status: Open

`item_references_seam` (`src/builtins/strings.rs:320-327`) only inspects
`Item::Function` and `Item::Binding`, so a scalar-seam reference
(`toScalars`/`fromScalars`/`isLetter`/…) that appears **only** inside another
item kind (notably `Item::Testing` TCASE bodies) does not trigger injection of
`strings_package.mfb`, leaving `__strings_*` undefined.

Currently not reachable in `mfb build`/`mfb test` because `lower_testing_blocks`
(`build.rs:327`) desugars TCASE bodies into `Item::Function`s before the
augmentation gate runs, and build mode drops testing blocks — so this is latent,
saved only by pass ordering. A new expression-bearing `Item` variant, or
reordering the desugar after augmentation, would break injection.

Fix: make `item_references_seam` also walk `Item::Testing` case bodies (and any
future expression-bearing items), or document/assert the pass-ordering invariant
(over-injection is harmless per the module's design note).
