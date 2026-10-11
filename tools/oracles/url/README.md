# oracles/url — MFBASIC's URL handling against Node's WHATWG URL and llhttp

`net::toUrl`, `toString(Url)`, `net::percentDecode`, `net::parseQuery` and the
`encoding` package's percent/form codecs are MFBASIC source with no second opinion
of their own, and the `http` client and server de-chunk bodies with their own
`__http_dechunkBytes`. This oracle is that second opinion: every case is played
against Node's WHATWG `URL`, `URLSearchParams`, `decodeURIComponent` /
`encodeURIComponent`, and against llhttp through `node:http`'s client and server.

**Offline tooling only.** Nothing here is linked into the compiler or runtime, and
nothing here runs in CI. All sockets bind or connect to 127.0.0.1.

## Run

```
tools/oracles/url/run.sh [path-to-mfb] [mode ...]
```

With no modes, every default mode runs. With no `mfb` path it uses `target/release/mfb`
(building it if missing). The subject is built from a copy under `$TMPDIR`, never in
the repo. Exit codes, shared with `tools/oracles/crypto/`: `0` every disagreement is in a
class-b group, `1` a class a / c / unclassified disagreement, `2` the harness could not
run (a build failed, a side died, or a case count was wrong).

Needs Node ≥ 22 (measured on 24.12.0). No npm install.

| Variable | Effect |
|---|---|
| `MFB_URL_WPT=<urltestdata.json>` | also run the `wpt` mode. The file is not vendored; the pinned copy is the one the `url` 2.5.8 crate ships (`~/.cargo/registry/src/*/url-2.5.8/tests/urltestdata.json`), checked by sha256 in `run.sh` |
| `MFB_URL_KEEP=<dir>` | copy every case list, both sides' answers and a JSON report per mode there |

## Modes

| Mode | Cases | What is compared |
|---|---|---|
| `parse` | 1955: `corpus/parse.txt` (429 hand-written, WPT style: schemes, userinfo, hosts, IDNA, IPv4/IPv6, ports, path, query, fragment), every ASCII byte and 14 Unicode code points in 10 URL positions, 32 ports, 60 IPv4 numeric forms, 14 long inputs | `net::toUrl`'s eight fields, `toString(Url)` and whether it re-parses to the same fields, against `new URL()` |
| `wpt` | 746: every distinct `input` of the pinned `urltestdata.json`, parsed without its base | as `parse` |
| `pdecode` | 613: `corpus/pdecode.txt`, all 256 `%XX`, 225 `%` pairs over a hex/non-hex alphabet, `encodeURIComponent` of the encode corpus | `net::percentDecode` and `encoding::percentDecode` against `decodeURIComponent`; `encoding::formUrlDecode` against the WHATWG form decoder |
| `query` | 202: `corpus/query.txt` and every ASCII byte in a key and a value | `net::parseQuery` against `URLSearchParams`, duplicates collapsed last-wins |
| `encode` | 168: `corpus/encode.txt`, ASCII, 9 Unicode code points | `encoding::percentEncode` against `encodeURIComponent`, `encoding::formUrlEncode` against URLSearchParams serialization, and that Node's decoder recovers the input |
| `chunk-client` | 49: `corpus/chunked.txt` as response bodies from a raw-socket server | `http::read`'s body or error against Node's `http.get` |
| `chunk-server` | 49: the same bodies as request bodies from a raw-socket client | `http::server`'s `Request.body` (or 4xx) against `http.createServer` (or `clientError`) |

## How it fits together

| Piece | Role |
|---|---|
| `corpus/*.txt` | Hand-written cases, one JSON literal per line |
| `node/gen.mjs <mode> <dir>` | Builds the case list (corpus + generated matrices), writes `<mode>.cases.json` and the subject's `<mode>.job`, prints the count |
| `mfb/` | The subject: reads the job (one hex line per case), prints `case <index> <fields...>`; each String is `x` + UTF-8 hex |
| `node/oracle.mjs <mode> <cases>` | The judge for the string modes, same line shape |
| `node/chunked.mjs client\|server ...` | Runs both sides of a chunk mode over 127.0.0.1 |
| `node/compare.mjs` | Sorts every disagreement into a named group and prints the groups, each with its class and shortest examples |
| `run.sh` | Builds the subject, runs every mode, checks the declared case counts |

The subject and the judge share no answers: both read the same case list. The expected
case count per mode is declared in `run.sh` (`expected_cases`), never counted from a
side's output.

## Divergence classes

`compare.mjs` assigns every disagreement to a group by a **rule** (never by case index),
and `GROUPS` there declares each group's class:

- **a** — MFBASIC disagrees with its own spec and the spec agrees with Node: a bug. Fails the run.
- **b** — a difference the spec allows: MFBASIC is deliberately narrower (http/https only,
  `scheme://` required, strict decoders) or deliberately does not normalize (no host
  lowercasing, IDNA, percent-encoding, dot-segment removal, tab/newline stripping). Counted,
  never fails the run.
- **c** — the spec is silent; an owner decision is needed. Fails the run until decided.

A group not in `GROUPS` is unclassified and fails the run. When a decision moves a group
from c to b (spec updated) or a fix empties a group, edit its `GROUPS` entry.

Class-b groups and their spec basis:

| Group | Spec basis |
|---|---|
| `parse/out-of-scope-scheme` | 06_url step 1: only `http` / `https` |
| `parse/strict/missing-slashes`, `parse/strict/empty-host-after-slashes` | 06_url grammar `scheme "://" authority` |
| `parse/strict/scheme-with-tab-newline-or-c0`, `parse/strict/port-not-digits`, `parse/input-cleanup` | 06_url: no input cleanup; WHATWG strips tab/LF/CR and outer C0/space |
| `parse/normalized-only/<fields>` | 06_url / `mfb man net toUrl`: no normalization. A case lands here only when WHATWG's normalization of each MFBASIC field, placed alone in a fixed URL, equals Node's parse of the whole input |
| `pdecode/form/strict` | 08_encoding: form decoding fails on a malformed escape or invalid UTF-8 |
| `query/raw-fallback` | 06_url: a malformed component is kept raw |
| `encode/percent/escapes-more`, `encode/form/escapes-more` | 08_encoding escape sets (only unreserved / only alphanumerics pass) |
| `chunk/te-substring`, `chunk/te-with-content-length` | 05_http: the client matches the substring `chunked`; the client-side parser stays lenient |

Every run prints each group with its class, count and shortest examples; `MFB_URL_KEEP`
keeps the full per-mode reports.

## Adding a mode

Add the mode in all five places, or `run.sh` exits 2:

1. `node/gen.mjs` — a `MODES` entry returning the case list.
2. `node/oracle.mjs` — a `JUDGES` entry (or `node/chunked.mjs` for a socket mode).
3. `mfb/src/main.mfb` — a branch printing the same fields.
4. `node/compare.mjs` — a `CLASSIFY` entry and its `GROUPS`.
5. `run.sh` — the name in `ALL_MODES` and its count in `expected_cases`.
