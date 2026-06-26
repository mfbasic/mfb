# Symbols And Relocations

Native code uses three broad kinds of symbols:

- Internal text symbols for app functions, package exports, and runtime helpers.
- Internal data symbols for constants and runtime data.
- External import symbols for platform libraries.

Relocations describe how instructions should be patched once final addresses are
known. Current aarch64 backends support:

- `branch26` for internal and external calls.
- `page21` and `pageoff12` for internal data addressing.
- Target-specific external data addressing where required.

External calls branch to linker-generated import stubs. The stubs load the
resolved dynamic address from a GOT-style table and branch to it.
