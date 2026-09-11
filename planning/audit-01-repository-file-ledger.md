# Repository file-by-file security review ledger

Scope: every tracked file below `repository/` at the review start, enumerated by
`rg --files repository | sort` on 2026-09-11. Tests are reviewed as evidence
for reachable production behavior; a passing test is not treated as a security
verdict.

| File | Review focus | Outcome |
|---|---|---|
| `Cargo.toml` | dependency/features and S3 activation | reviewed; no code-level finding |
| `DEPLOY.md` | operator trust, key and deployment instructions | reviewed; feeds bug-586 remediation |
| `Dockerfile` | runtime UID, persistent paths, defaults | finding: bug-586 |
| `docker-entrypoint.sh` | environment-to-argument handling and secret disclosure | reviewed; no finding |
| `fly.toml` | listener, service and volume exposure | reviewed; no finding |
| `src/abi.rs` | hostile MFPC parsing and allocation bounds | finding: bug-578 |
| `src/backfill.rs` | operator rewrite safety and signed metadata provenance | reviewed; no finding |
| `src/blobstore.rs` | local/S3 object addressing, staging, presigning | reviewed; redirect consumer feeds bug-585 |
| `src/client.rs` | transport, redirects, pins, signatures and key lifecycle | findings: bugs 581, 582, 585 |
| `src/crypto.rs` | randomness, domain separation, pairing encryption | reviewed; pairing protocol feeds bug-583 |
| `src/gc.rs` | deletion reachability, grace and race recheck | reviewed; no finding |
| `src/lib.rs` | module/API boundary | reviewed; no finding |
| `src/local.rs` | client secret permissions and pin persistence | reviewed; contrasts with bug-586 |
| `src/log.rs` | Merkle inclusion/consistency verification | reviewed; unsafe caller behavior is bug-582 |
| `src/main.rs` | operator ceremony parsing and root lifecycle | finding: bug-584 |
| `src/package.rs` | artifact framing, signature/proof/attestation binding | reviewed; no additional finding |
| `src/server.rs` | route authz, request limits, key transitions, publication | findings: bugs 579, 583; supports 580–584 |
| `src/store.rs` | authorization persistence, transactions, secret storage | findings: bugs 580, 583, 584, 586 |
| `src/terminal_safe.rs` | terminal-control boundary | reviewed; no finding |
| `src/validation.rs` | names, identifiers and query/path inputs | reviewed; no finding |
| `src/web/mod.rs` | XSS, CSP, link scheme filtering and HTML escaping | reviewed; no finding |
| `src/web/style.css` | static presentation asset | reviewed; no executable/security-sensitive behavior |
| `tests/s3_backend.rs` | S3 backend behavior evidence | reviewed; no additional finding |

## Filed findings

- 578 HIGH — parser allocation amplification.
- 579 MEDIUM — anonymous transparency-log work is unbounded.
- 580 LOW — auth and ident credentials may be the same key.
- 581 HIGH — index response is not bound to requested identity/signed metadata.
- 582 HIGH — a larger log fork overwrites a client checkpoint without proof.
- 583 MEDIUM — a relay-visible pairing lookup can enroll an auth key.
- 584 MEDIUM — root replacement has no authenticated continuity.
- 585 MEDIUM — hostname redirects can resolve to internal SSRF targets.
- 586 MEDIUM — default container database permissions expose private keys.

This ledger records coverage and findings, not a claim that future code changes
or a different deployment topology are secure.
