# bug-489: registry-authored response strings render to the terminal unsanitized, forging the `[Verified]` trust line

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM
Class: security (terminal spoofing / trust-decision forgery)

Status: **FIXED** (2026-09-05)

Regression Test: `repository/src/client.rs` — `a_server_authored_error_is_escaped_at_the_boundary`,
`a_server_authored_newline_is_escaped`, `an_ordinary_server_error_is_unchanged`;
and `tests/cli_untrusted_registry_text_is_escaped.rs`, which pins the half of the
design that is easy to undo.

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

## Best fix — as proposed here it BREAKS 19 messages; corrected below

**The print-site fix in this section is wrong, and it was tried.** Wrapping
`message` in `dispatch_command_error` mangles the compiler's own output:

    $ mfb org        # with the print-site wrap applied
    error: mfb org grant <org> <member> <owner|admin|publisher> [--as <grantor>]\u{000a}       mfb org remove <org> <member> [--as <grantor>]

`terminal_safe::safe` escapes `\n` — **correctly**, because a server-authored
newline forges whole rows, which is the same forgery this bug is about one line
down. But 19 `CommandError` messages carry deliberate newlines:

    $ grep -rn 'CommandError::\(Failed\|Usage\)' -A3 src/ | grep -c '\\n'
    19

They are the multi-line usage hints (`mfb repo …\n\n<hint>`, the `org`/`token`
two-line synopses). Every one of them would ship as a single
`\u{000a}`-littered line.

**The print site cannot tell trusted from untrusted** — that is the whole
problem, and no cleverness at the printer fixes it. A `\n` from the compiler is
legitimate; a `\n` from a registry is an attack. Only the source knows which.

### What was done instead

Sanitize at the **trust boundary**, and split on a property that is actually
decidable there:

* **Error strings are display-only by construction**, so they are sanitized where
  they are created — the six places in `mfb_repository::client` where a
  server-authored string becomes an `Err(String)`: three `ErrorResponse.error`
  returns and three `"…failed with status {status}: {text}"` fallbacks, which
  interpolate the raw response body and were not in this document's list.
  `sanitize_server_text` carries the reasoning. This protects all **53** `mfb`
  sites that consume a client result, including ones added later — sanitizing at
  each of those instead is the same "two lists that must agree" failure the fix
  exists to close.
* **Data fields may be COMPARED or stored**, so escaping them at the source would
  change program logic rather than display. Those are sanitized at their print
  sites: `pkg.rs` `Release State`, `resolve.rs`'s `Installed`/`+`/`~`/` `/`-`
  lines, `repo.rs`'s link/grant/remove confirmations, and `pkg.rs`'s
  `ident:`/`version:` pair.

`terminal_safe` itself moved to `mfb_repository::terminal_safe` (`mfb` re-exports
it) because the client is now a caller and `mfb_repository` cannot depend on
`mfb`. One implementation, one set of tests — a sanitizer that escaped different
sets on each side of the crate boundary would be worse than either alone.

### The census is bounded — say so rather than claim completeness

The print sites above were found by grepping `src/cli/` for `println!`/`eprintln!`
interpolating a response field. That grep cannot see a field reached through a
local binding or a helper, so **this is not a proof of completeness**, and it
found more sites than this document originally listed (`repo.rs` and
`resolve.rs`'s summary lines, `pkg.rs:1465`). The durable fix for the remaining
tail is structural rather than another census: give the client's display-only
fields a `ServerText` newtype whose `Display` sanitizes, so a raw `{}` cannot
compile. That is a wider refactor across the response structs and is not done
here.

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
