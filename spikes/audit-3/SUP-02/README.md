# SUP-02 spike — registry terminal-injection forges a `[Verified]` line

audit-3 finding SUP-02 (`planning/audit-3-supply-chain.md`), bug-489.

Not an MFB program: the untrusted input is an HTTP response body, not a `.mfb`
source. This is the command repro from the finding, checked in so it can be
re-run.

## Run

```
# 1. Start the malicious registry (answers GET /ident with a hostile error body):
python3 evil-registry.py &          # listens on 127.0.0.1:7799

# 2. From the scratch project, ask mfb to add a package from it:
cd project
MFB_HOME=/tmp/sup-home MFB_REPO_URL=http://127.0.0.1:7799 \
  mfb pkg add 'alice#toolbox' > /tmp/sup-out.txt 2>&1

# 3. Inspect the raw bytes mfb printed:
cat -v /tmp/sup-out.txt
```

## Observed (defect present)

```
error: ^[[2K^Mok: uses toolbox - [Verified]  M-bM-^@M-.EVIL
```

`^[[2K^M` is ESC-`[2K` (erase line) + CR, so on a real terminal the `error: `
prefix is wiped and the line renders as an apparent success. `M-bM-^@M-.` is
U+202E (right-to-left override) — exactly the class `src/terminal_safe.rs::safe`
escapes for `.mfp` header fields but does not apply to registry responses.

## Expected (after the fix)

Every control byte rendered as `\u{XXXX}`, the way `mfb pkg info` already renders
the same bytes coming from a `.mfp` header — e.g.
`error: \u{1b}[2K\rok: uses toolbox - [Verified]  \u{202e}EVIL`.
