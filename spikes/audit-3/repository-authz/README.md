# Registry authorization exploits — audit-3 REPO-01/02/03 (bug-492/493/494)

Not MFB programs: the untrusted surface is the `mfb-repo` HTTP API, so these are
plain HTTP clients. Each opens the server's SQLite store directly to register an
account (standing in for `mfb repo register`, which needs interactive key
generation), then drives the exploit over HTTP. Re-run by the lead 2026-09-03.

## Setup — run the server against a scratch DB

```
cd <repo-root>/repository
cargo build --bin mfb-repo
rm -rf /tmp/repo-audit3 && mkdir -p /tmp/repo-audit3/blobs
../target/debug/mfb-repo --dbpath /tmp/repo-audit3/meta.db \
    --datapath /tmp/repo-audit3/blobs --listen 127.0.0.1:8791 &
```

The harness bins hard-code `http://127.0.0.1:8791` and
`/tmp/repo-audit3/{meta.db,blobs}`.

## REPO-01 (bug-492) — scoped publish token self-escalates to unscoped signing

```
cargo run --bin tokenesc -- orgB1
```

Observed (defect present):
```
token /signing orgB1#ci-only  -> 200 OK
token /signing orgB1#flagship -> 400 Bad Request     # scope enforced for the token
new UNSCOPED auth key fp = ed564f2e…                 # via /machines/link (self-pairing)
CI token revoked = true                              # owner revokes the token
escalated key /signing orgB1#flagship -> 200 OK      # STILL signs flagship, post-revoke
```

## REPO-02 (bug-493) — an auth key revokes another auth key (ident key never used)

```
cargo run --bin revoke -- victimB1
```

Observed:
```
ident private key is NEVER used below
revoke status = 200 OK
primary key still usable for login? false            # a secondary auth key killed the primary
```

## REPO-03 (bug-494) — former owner keeps yank + attest rights after a transfer

```
cargo run --bin transfer -- t2b
```

Observed:
```
package owner after transfer = Some("bobt2b")
former-owner yank status = 200 OK                    # alicet2b yanks a package she gave away
former-owner /signing status = 200 OK                # ...and still gets a signed attestation
current-owner unyank status = 400 Bad Request        # bobt2b, the real owner, is locked out
```

## REPO-09 (bug not filed — LOW) — expired publish token still logs in

```
cargo run --bin expired
```

Demonstrates the token expiry is enforced only at `/signing`, not at
`/auth/login` or `PUT /blob`.

Expected, after the fixes: every "defect present" line above becomes a refusal
(400/403), and the real owner's operations succeed.
