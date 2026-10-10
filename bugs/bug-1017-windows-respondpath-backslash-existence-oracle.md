# bug-1017: on Windows, `http::respondPath` still answers 403 vs 404 by existence for a `..\` traversal (bug-862's oracle, through a backslash)

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (proposed) a Windows row for `func_http_respondPath_valid`, or a unit test of `__http_pathEscapesRoot` with `\` separators, run on box 206

bug-862 made `http::respondPath` reject a `..` escape lexically, before any filesystem probe, so a
traversal is `403` whether or not its target exists. The lexical check (`__http_pathEscapesRoot`)
splits the relative path on `/` only. Windows also accepts `\` as a separator, so `..\name` is one
segment to the check, passes it, and reaches `fs::fileExists`: an existing target is then `403`
(the containment check) and a missing one `404`. A remote client can probe for any file on the
host.

**Correct behavior:** on Windows a `\` separates path segments in the escape check too, so every
escaping path is `403` regardless of existence. On POSIX a `\` is an ordinary filename character
and must stay one.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on box 206 (Windows 11), built with
`mfb build -target windows-x86_64` by bs18-agent-i's compiler (which has bug-862 and bug-983).
The program builds an `http::Request` whose `*` capture is the probe string and prints
`http::respondPath(req, root).status`; `root` is `<temp>\bs18rev_root`, and
`<temp>\bs18rev_secret.txt` exists.

| `*` capture | status |
|---|---|
| `hi.txt` (in root) | 200 |
| `../bs18rev_secret.txt` (exists) | 403 |
| `../bs18rev_missing.txt` | 403 |
| `..\bs18rev_secret.txt` (exists) | **403** |
| `..\bs18rev_missing.txt` | **404** |

Expected: `403` for both backslash rows.

## Related

- bug-862 (the `/` form, fixed). Suspected by bs18-agent-i; reproduced by the coordinator.
