# bug-210: untrusted package/manifest strings rendered to the terminal without control/bidi sanitization

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: security (terminal spoofing)

Status: Open

Two code paths print strings sourced from an untrusted `.mfp`/manifest verbatim
to the operator's terminal — the exact "forge the report used to decide trust"
threat — without escaping control/ESC/newline or Unicode bidi/format overrides:

- `src/audit/text.rs:14-27` (`safe`) escapes only C0/C1 control chars
  (`char::is_control`) and lets bidi/format overrides through (U+202E RLO,
  U+200E/200F, U+2066–2069, zero-width), permitting visual reordering of audit
  rows (e.g. a dependency name `legit\u{202e}drowssap.mfp`).
- `src/cli/pkg.rs:1033` (and `validate_package_file` ~659, `verify_packages`
  ~870) print `.mfp` header fields (author/url/ident/proof/attestation/name)
  verbatim; `read_mfp_string` enforces only valid UTF-8, so ESC/`\r`/`\n` in a
  crafted package can overwrite/recolor the terminal or forge a "result: valid"
  line. Same class as completed bug-24 but different locations (cli + bidi gap),
  not covered by bug-24's fix.

Trigger: `mfb audit` / `mfb pkg info evil.mfp` on a project/package whose
name/author/url contains ESC/newline/bidi overrides → spoofed trust output.

Fix: route all externally-sourced strings through one sanitizer that escapes
C0/C1 **and** bidi/format code points (general-category Cf plus the RLO/LRO/PDI
ranges) and rejects embedded newlines, at both the audit and cli print sites.
