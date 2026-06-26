# Failure Rules

The linker must fail rather than generate a broken executable when:

- An internal symbol cannot be resolved.
- A relocation kind is unsupported for the target.
- An external relocation references an import that has no generated stub.
- A target receives an import for a library it does not support.
- A required package export or ABI dependency cannot be resolved.

It is not valid to satisfy missing linker support with a placeholder helper, an
empty dynamic dependency, a zero address, or a runtime "unsupported" fallback.
