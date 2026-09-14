# tools/security-package-sources

Generators for the deliberately malformed package (`.mfp`) fixtures behind
`tests/rt-behavior/security`. Each `pkg-0N-<slug>/generate.py` writes one tampered
package (a bad signature, type confusion, decode depth, a type cycle, an allocation
count, a duplicate section, a need overflow, a parameter's default-function record
pointing at the wrong function), using the shared helpers in
`mfp_craft.py`. The security fixtures prove the loader refuses each one.

These are NOT rebuilt by `scripts/sync-package-mfp.sh` (a normal rebuild would undo
the tampering). Regenerate one after a container-format change with:

    python3 tools/security-package-sources/<pkg>/generate.py

`tests/rt-behavior/security/README.md` describes the fixtures that consume them.
