# bug-420: registry LOW-severity security findings (goal-07 batch) — org admin privilege escalation, client redirect SSRF, anonymous fuzzy-search DoS

Last updated: 2026-07-28
Effort: small (<1h) each
Severity: LOW
Class: Security (authorization / SSRF / algorithmic-DoS)

Status: Open
Regression Test: per item (see each).

Three LOW-severity `mfb-repo` security findings from the goal-07 review, batched
(all registry-security, LOW). Each has a distinct fix; kept together for triage.

## Items

### (1) `repository/src/server.rs:1697` + `store.rs:2010` — org admin → owner self-promotion
`/orgs/members` authorizes a grant/remove when the grantor is the org itself OR holds
role `owner`/`admin` (`server.rs:1697`:
`!is_org && !matches!(grantor_role, Some("owner")|Some("admin"))`). But
`store.rs grant_org_member`/`remove_org_member` (`:2010`/`:2049`) impose **no
role-transition guard** — they only validate the role string is one of
`owner|admin|publisher`. So an `admin` member can grant role `owner` to themselves
and/or remove the org's existing owner(s), seizing full control of the org
namespace. Requires already being an admin (semi-trusted), so escalation-from-admin,
not anonymous — hence LOW.
- Fix: forbid a non-owner grantor from granting/removing the `owner` role (only an
  owner or the org itself may manage owners). Regression test: an admin cannot
  grant owner or remove the last owner.

### (2) `repository/src/client.rs:65` — unrestricted redirect following (SSRF / transport downgrade)
The shared `reqwest` Client is built with timeouts but **no `.redirect(...)`
policy**, so it follows up to 10 redirects to any host/scheme;
`ensure_transport_security` validates only the initial `repo_url`, never a redirect
target. A malicious/compromised registry can answer `/blob`, `/index`, `/log/*`,
`/root.json` with a 302 to an internal/link-local host (169.254.169.254, 127.0.0.1,
RFC-1918) or a plaintext `http://` URL. Result: (a) blind SSRF from a dev/CI machine
at internal services; (b) a redirect to `http://` leaks which blob/package is being
fetched in cleartext despite a pinned https registry. **Authenticity is NOT
affected** — blob bytes stay SHA-256-checked (`:1185`), control-plane bodies stay
signature-checked; the bearer token is stripped on cross-origin redirect. Hence LOW.
(The comment at `client.rs:1332-1334` already acknowledges silently following a 302
to a presigned-URL host.)
- Fix: set an explicit redirect policy — reject cross-scheme downgrade (https→http),
  and re-apply `ensure_transport_security` (host/scheme allowlist, block
  private/link-local) to each redirect target.

### (3) `repository/src/store.rs:1592` — anonymous fuzzy-search full-table scan (algorithmic DoS)
`search_packages`'s fuzzy tail runs `SELECT p.ident, o.owner_display FROM packages p
JOIN owners o` — an unindexed full-table scan — then allocates a `Vec<char>` per
ident and runs `within_edit_distance_one` for every package (`:1592-1614`). It fires
whenever the ranked query returns zero first-page rows (`idents.is_empty() && offset
== 0`), which an anonymous client forces with a gibberish query. This reintroduces
the O(n) scan+Levenshtein cost the module deliberately gated `typosquat_candidates`
out of the hot path to avoid (doc at `:1503-1508`). Mitigated (120 req/60s/IP,
first-page only, cheap length pre-filter) → latent amplification, worst on a large
registry. Hence LOW.
- Fix: gate the fuzzy tail behind a minimum query length / a bounded candidate set,
  or an FTS/trigram index; don't full-scan on every no-match anonymous query.

References: `repository/src/server.rs:1697`, `store.rs:2010`/`:2049`/`:1592`,
`client.rs:65`/`:1332`. goal-05's audit-2/goal-05 checked these areas but did not
catch admin→owner self-promotion, redirect-target restriction, or the fuzzy-tail
scan. Found during goal-07.

## Failing Reproduction

All require a live registry/network harness (not run); each is confirmed statically
from the cited source (see items).

## Goal / Non-goals / Blast Radius

Per item above. Each fix is local to its cited site(s); none changes the correct
authenticity/verification paths (blob hash + signature checks stay intact).
