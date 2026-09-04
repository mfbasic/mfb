# bug-489: registry-authored response strings render to the terminal unsanitized, forging the `[Verified]` trust line

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM
Class: security (terminal spoofing / trust-decision forgery)

Status: Open (found in audit-3, Surface 9 SUP-02; `planning/completed/audit-3-supply-chain.md`)

Regression Test: none yet — add one asserting a registry error carrying ESC/CR/U+202E renders `\u{XXXX}`-escaped.

## Summary

The compiler-side registry client returns the server's own free-form `error`
string to the CLI verbatim, and the CLI prints it to the operator's terminal
with no control/bidi sanitization. A malicious or MITM'd registry (anything the
operator points `MFB_REPO_URL` at, including the default) can therefore author a
response that, on a real terminal, erases the `error:` prefix and renders as an
apparent `[Verified]` success — the exact "forge the report the operator uses to
decide whether to trust a package" threat that `src/terminal_safe.rs` was
written for. This is the same class as completed **bug-24** and **bug-210**, at a
new untrusted source (the registry response) that their censuses did not cover.

## Mechanism

`read_json_response` hands back the server string unchanged:

```rust
// repository/src/client.rs:1471
if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
    return Err(error.error);
}
```

`ErrorResponse.error` is a free-form `String` bounded only by `MAX_JSON_BYTES`
(`repository/src/server.rs:690`). It flows to the CLI's error printers with no
escaping:

- `src/cli/mod.rs:32,36` — `eprintln!("error: {message}")`
- `src/rules/mod.rs:104` — `eprintln!("               {}", detailed_message)`

and the same gap exists for other registry-sourced fields printed raw:
`src/cli/pkg.rs:1874` (`println!("Release State: {}", version.state)`),
`src/cli/resolve.rs:436-437`.

`src/terminal_safe.rs::safe` escapes exactly the dangerous set (C0/C1 controls +
the bidi/format code points), but a census confirms it is applied only to `.mfp`
header fields, never to a registry response:

```
$ grep -rn 'terminal_safe' src/
src/cli/pkg.rs:1837,1839,1847,1855,2036
src/audit/text.rs:9,12
src/audit/json.rs:92
src/main.rs:29 (mod)
```

## Reproduction (run against `target/debug/mfb`)

`spikes/audit-3/SUP-02/` carries the harness. In brief:

```
python3 spikes/audit-3/SUP-02/evil-registry.py &     # 127.0.0.1:7799, GET /ident -> hostile error
cd spikes/audit-3/SUP-02/project
MFB_HOME=/tmp/sup-home MFB_REPO_URL=http://127.0.0.1:7799 \
  mfb pkg add 'alice#toolbox' > /tmp/sup-out.txt 2>&1
cat -v /tmp/sup-out.txt
```

The registry answers `GET /ident` with
`{"error":"\x1b[2K\rok: uses toolbox - [Verified]  ‮EVIL"}`.

- **Observed:** `error: ^[[2K^Mok: uses toolbox - [Verified]  M-bM-^@M-.EVIL` —
  raw ESC-`[2K` (erase line) + CR (wipes `error: `) and U+202E RLO.
- **Expected:** every control byte rendered `\u{XXXX}`, as `mfb pkg info`
  already renders the same bytes from a `.mfp` header.

## Best fix

Route every externally-sourced string through `terminal_safe::safe` **at the
print site** so the sanitizer stays the single choke point: wrap `message` in
`dispatch_command_error` (`src/cli/mod.rs:32,36`), `detailed_message` in
`show_general_diagnostic` (`src/rules/mod.rs:104`), and the registry-sourced
operands at `src/cli/pkg.rs:1874` and `src/cli/resolve.rs:436-437`. Add a test
asserting an ESC/`\u{202e}`-bearing registry error renders escaped so the census
does not silently regress again.

## Non-goals

- Do not change the wire format of `ErrorResponse`.
- Escape, do not truncate — the message must still be readable.
- Do not alter the diagnostic rule codes or the `severity[code NAME]:` header
  shape that goldens pin.

## Prior art

Extends **bug-24** (`bugs/completed/bug-24-audit-text-terminal-injection.md`)
and **bug-210** (`bugs/completed/bug-210-untrusted-string-terminal-rendering.md`,
Fixed 2026-07-15, scoped to `.mfp` header fields + `audit/text.rs`). No prior
item covers the registry-response source.
