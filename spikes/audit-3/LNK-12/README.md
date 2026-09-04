# LNK-12 spike — project.json `name` path-joined unsanitized → arbitrary 0755 executable write

audit-3 LNK-12 (`planning/audit-3-linker-hardening.md`), bug-503. Building an
untrusted project should never write outside it (`mfb build` does not even run the
program), but the project `name` is `Path::join`ed into the output path with no
component validation — `..` escapes and a leading `/` is absolute.

```
mkdir -p /tmp/lnk12-pwn
mfb build spikes/audit-3/LNK-12
ls -l /tmp/lnk12-pwn/          # evil.out, a 0755 Mach-O, written OUTSIDE the project
```

## Observed (defect present)

```
Wrote executable to /tmp/lnk12/build/../../../../../../tmp/lnk12-pwn/evil.out
/tmp/lnk12-pwn/evil.out: Mach-O 64-bit executable arm64   (mode 0755)
```

## Expected

`mfb build` rejects the name with the existing `validate_package_name` check
(applied on the package path but not the executable path).
