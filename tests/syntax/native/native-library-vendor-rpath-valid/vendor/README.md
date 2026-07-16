# `libfixture.so` is **not** a shared object

It is a plain text file, deliberately.

`plan-46-B`'s vendor-hash check reads `<project root>/vendor/<source>` and
records its sha256 into the `.mfp`'s `NATIVE_LIBRARY_TABLE`. That check does not
care whether the file is a *valid* ELF — only that its bytes hash stably. A few
committed bytes therefore give a hermetic golden with no toolchain dependency on
the test host, which building a real per-arch `.so` would require.

Do not "fix" this into a real shared object. Nothing in this fixture ever loads
it: the package is only built, never linked into an executable and never
`dlopen`ed. Replacing it would churn the golden hash for no gain.
