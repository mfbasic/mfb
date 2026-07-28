# bug-210: untrusted package/manifest strings rendered to the terminal without control/bidi sanitization

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: security (terminal spoofing)

Status: Fixed (2026-07-15) — added one shared sanitizer, src/terminal_safe.rs, that escapes C0/C1 controls AND the invisible-but-active Unicode bidi/format set (U+061C, U+200B-200F, U+202A-202E, U+2060-2064, U+2066-2069, U+FEFF) as \u{XXXX}; well-formed values pass through borrowed. audit/text.rs's safe() now delegates to it (it previously escaped only char::is_control, letting U+202E RLO through and permitting visual reordering of audit rows). cli/pkg.rs's print_package_info now routes every untrusted .mfp header field through it: empty_marker() sanitizes (ident/identKey/signingKey/author/url) and name/version/proof/attestation are wrapped directly. Regression Test: terminal_safe unit tests (4) cover C0/C1, each bidi/format class, and non-ASCII pass-through; verified end-to-end that `mfb audit` on a project whose name/version carry U+202E and ESC renders them escaped with 0 raw dangerous bytes. 38 pkg + 76 audit tests pass.

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
