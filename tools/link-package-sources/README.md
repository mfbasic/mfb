# tools/link-package-sources

Source projects for the native-link-collision package fixtures: `collidera` and
`colliderb`, two packages built to collide at link time so the tests can prove the
linker keeps them apart.

Their compiled `.mfp` copies are what the fixtures consume. Rebuild them from these
sources with `scripts/sync-package-mfp.sh` whenever the package binary format
changes; a stale copy is silently mis-lowered by a newer compiler.
