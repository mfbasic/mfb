# Extension Checklist

The checklist of every layer to update when adding an end-to-end language feature.

When adding a language feature or built-in that must work end to end, update
every layer that observes or emits that behavior:

1. Lexer/parser support, if syntax changes.
2. AST model and AST serialization.
3. Resolver rules for names, imports, constructors, and members.
4. Monomorphization rules for generic forms.
5. Type-checking rules and overload behavior.
6. IR lowering.
7. MFPC binary representation lowering, encoding, decoding, exports, and ABI hashing.
8. NIR lowering for native builds.
9. Runtime helper detection and native backend capabilities.
10. Native plan and native code-plan lowering.
11. AArch64 encoding, if new instruction forms are needed.
12. OS linker/container support, if relocations/imports/layout change.
13. Package dependency merge/import behavior, if packages are affected.
14. Valid and invalid function tests for every changed public function.
15. Acceptance suite updates only after proving mismatches are expected.
16. Runtime validation for executable behavior, not just generated artifacts.

This checklist follows the repository's completion rule: compiler output alone
does not prove a runtime feature works. Executable behavior must be validated by
running the generated program or by another observable runtime result.

## See Also

* ./mfb spec architecture frontend — the resolver, monomorphization, and checker layers steps 3–5 touch
* ./mfb spec architecture flows — the end-to-end build sequence these layers compose into
* ./mfb spec package binary-representation — the MFPC lowering and encoding step 7 updates
* ./mfb spec package verifier-rules — the IR semantic rules a new feature must satisfy
