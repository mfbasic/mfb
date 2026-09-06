# oracle — differential-test `packages/jwt` against an independent JOSE implementation

The package's own tests (`mfb test packages/jwt`) pin the behaviour this project
decided on. They cannot catch a *misreading* of RFC 7515/7518/7519, because the
same misreading is in the code and in the test. This directory is the other
half: the same questions, asked of an implementation someone else wrote from the
same specifications.

```sh
npm install          # once
./run.sh             # build everything, then every mode
```

Exit status is 0 iff every case agreed, or diverged for a reason declared in
`divergences.json`.

## The oracle

[`jose`](https://www.npmjs.com/package/jose) (panva/jose) — the JOSE
implementation the Node ecosystem actually runs, and the one most JWT libraries
are checked against.

**Version 5, not 6, on purpose.** jose 6 runs on WebCrypto, and WebCrypto has no
Ed448, so it dropped `EdDSA` over that curve. `packages/jwt` supports both RFC
8037 curves — `crypto` implements Ed448 in software — and an oracle without it
would leave a third of the signature surface unchecked. jose 5 reaches Node's
own `crypto`, which has Ed448.

## How the two sides talk

`probe/` is a small MFBASIC executable that answers a whole **job file** at once
and writes one JSON document back, in the same order:

```json
{"cases": [{"id":"c1","op":"sign","key":{...},"claims":{...}},
           {"id":"c2","op":"verify","token":"...","keys":[...],"options":{...}},
           {"id":"c3","op":"jwk","key":{...},"withPrivate":false}]}
```

```json
{"results": [{"id":"c1","ok":true,"token":"..."},
             {"id":"c2","ok":false,"code":93130001,"message":"jwt: the token expired at ..."}]}
```

`oracle.mjs` answers the identical file with jose. A process per question would
cost more than answering it, and a fuzzing run asks tens of thousands.

Two things make the comparison meaningful rather than incidental:

* **Keys cross as JWKs.** It is the one key format both sides already speak, and
  it puts `jwt::importJwk` on the hot path of every case instead of in a corner
  of the test suite.
* **The clock is pinned.** Every case carries `timeSeconds`, so a token that
  expires between the two runs cannot make them disagree, and a failure
  reproduces tomorrow.

A **refusal is a result**, reported inside the document with exit 0. That leaves
a non-zero exit or an unparseable document meaning exactly one thing — the probe
itself broke — which is what `mutate` mode checks for. Two refusals count as
agreement without comparing the reasons: the two implementations have their own
vocabularies for "no", and demanding the same words would make the harness fail
on wording.

## The four modes

| Mode | What it does | Asserts |
| --- | --- | --- |
| `corpus` | The hand-written cases in `corpus.mjs` — every algorithm, every registered claim at and around its boundary, every rejection rule, and JWK import/export. | Agreement, or a declared divergence |
| `cross` | Each side signs; the **other** side verifies. All eight key/curve combinations, both directions. | Mutual acceptance, and byte-identical tokens for the deterministic algorithms |
| `fuzz` | Random claims and options over every key, seeded so a failure reproduces. | Exact agreement |
| `mutate` | Valid tokens with random byte-level damage. | **Robustness** — a well-formed envelope whatever the damage — and agreement |

`cross` is the mode that earns its place. A round trip through our own verifier
would accept a signature format only we produce; handing an `ES512` token to
jose and an Ed448 token back the other way will not. It also compares the two
tokens **byte for byte** wherever the algorithm is deterministic — HMAC and RFC
8032 EdDSA are functions of key and message alone, so a difference of one bit is
a finding. ECDSA is randomized (`mfb man crypto sign`), so it is checked only by
cross-verification.

## Declared divergences

`divergences.json` names each case that must **not** agree, with the reason. A
declared case that stops diverging fails the run — that is how the harness
notices the package quietly changing a documented policy.

Every current entry is `packages/jwt` being deliberately stricter than jose:

| Case | jose | `packages/jwt` |
| --- | --- | --- |
| `strict/padded-payload` | accepts `=` padding on a segment | refuses — RFC 7515 §2 defines the JOSE encoding as the unpadded form |
| `strict/duplicate-alg`, `strict/duplicate-exp` | `JSON.parse` keeps the last of two same-named members | refuses — two readers need not keep the same one |
| `strict/short-hmac-secret` | accepts a secret shorter than the digest | refuses — RFC 7518 §3.2 requires at least the digest length |
| `jwk/marked-for-encryption` | ignores `use` on import | refuses `use: "enc"` for a signature key |
| `claims/numericdate/fractional-exp` | compares a fractional `exp` numerically | refuses — RFC 7519 §2 makes a NumericDate whole seconds |
| `claims/numericdate/huge-exp` | compares an `exp` past 2^53 as the Float it rounded to | refuses — the value that came back is not the value written |

They are all one idea: **a token should have exactly one text.** Where a byte
string has several legal spellings, anything keyed on the text — a replay cache,
a revocation list, an audit log — can be desynchronised from anything keyed on
the claims.

`mutate` mode reaches the same class from the other side, and does not take it
on trust. When jose accepts damage that `packages/jwt` refuses, the harness
re-encodes jose's own decode of each segment and requires the result to differ
from the token text — proving the token really was a non-canonical spelling. If
it round-trips unchanged, the token was canonical and our refusal is a failure,
not a policy. A 600-case run reports roughly four such spellings: an inserted
space inside a segment, or one extra symbol on the end, both of which
`Buffer.from(…, 'base64url')` reads straight through.

## What is not compared, and why

* **A verify answer's header.** Both sides echo the token's own header back, so
  comparing it would compare `JSON.parse` against `json::parse`.
* **A JWK's labelling.** `jwt::exportJwk` writes `alg` and `use: "sig"` and keeps
  the `kid`; jose's `exportJWK` writes the bare key parameters. Both are legal
  JWKs and neither policy is the other's bug — what has to match is the key
  material both read out of one JWK, so the comparison projects onto
  `kty`/`crv`/`x`/`y`/`d`/`k`.
* **`RS256`/`PS256` and the rest of the RSA family.** jose implements them and
  `packages/jwt` does not, because `crypto` has no RSA primitive. The corpus
  carries an `RS256` token so the refusal is exercised; both sides reject it (the
  token is not actually RSA-signed), so it is not a divergence — the gap is
  recorded here rather than pretended away.

## Layout

| Path | What it is |
| --- | --- |
| `oracle.mjs` | The oracle: a job file → the JSON envelope, via jose. Also importable (`runJob`, `runCase`). |
| `corpus.mjs` | The hand-written cases, and the helpers that craft a token over exact header/payload text. |
| `diff.mjs` | The runner: the four modes, and the comparison. |
| `divergences.json` | Case id → why it must diverge. |
| `keys.json` | The fixed test keys, one per algorithm and curve. |
| `probe/` | The MFBASIC side: `jwt::sign`/`verify`/`importJwk` → the same envelope. |
| `run.sh` | Build the package, the probe, run the package's tests, then the comparison. |

`keys.json` is committed on purpose: a corpus case has to be reproducible by
hand, and a freshly generated key makes yesterday's failure unrepeatable. They
are test keys and nothing else has ever signed with them.

`node_modules/`, `probe/packages/jwt.mfp` and `probe/build/` are git-ignored;
`package-lock.json` is tracked so `npm install` is reproducible.

## What it found

Building the probe found **bug-557**: an executable could not link a package
that called `crypto::hash`, `seal`/`open`, `sign`, `verify` or `generate`, because
those members' native dispatch branches to an MFBASIC helper by its bare reserved
symbol while `merge_packages` had renamed the package's copy. `packages/jwt` was
the first package to use those members, and the probe was the first executable to
import such a package without importing `crypto` itself.
