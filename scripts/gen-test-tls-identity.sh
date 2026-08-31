#!/usr/bin/env bash
# Generate a throwaway test CA and a 127.0.0.1 server identity for the local
# TLS loopback proof (plan-110-F Phase 1).
#
# Writes into <outdir>:
#   ca.pem          the test CA certificate (what a client must be told to trust)
#   ca-key.pem      the test CA private key
#   server.pem      the server leaf certificate, signed by that CA
#   server-key.pem  the server private key, PKCS#1 ("BEGIN RSA PRIVATE KEY")
#   server-key-pkcs8.pem
#                   the SAME key in PKCS#8 ("BEGIN PRIVATE KEY") -- see below
#   chain.pem       leaf followed by the CA, the form a server presents
#
# The leaf carries `subjectAltName = IP:127.0.0.1` so a client validating the
# name against the address it dialled succeeds. A CN alone is not enough for any
# modern verifier.
#
# ## Why two key encodings
#
# macOS routes `tls::listen`'s keyPath through `SecItemImport`, which returns
# `errSecUnknownFormat` (-25257) for a PKCS#8 `-----BEGIN PRIVATE KEY-----` and
# accepts only the traditional PKCS#1 `-----BEGIN RSA PRIVATE KEY-----` form --
# which is NOT what a modern `openssl req` emits by default. The PKCS#1 file is
# what the loopback proof uses; the PKCS#8 file is kept beside it as the exact
# reproduction for that defect (plan-110-D §C3, carried into plan-110-F Phase 2).
#
# Everything is regenerated on each run and is safe to delete. Nothing here is
# ever installed into a system trust store.
#
# Usage: gen-test-tls-identity.sh <outdir>
set -eu

if [ "$#" -lt 1 ]; then
  echo "usage: gen-test-tls-identity.sh <outdir>" >&2
  exit 2
fi
out=$1
mkdir -p "$out"

if ! command -v openssl >/dev/null 2>&1; then
  echo "FAIL: openssl not found; cannot generate a test identity" >&2
  exit 1
fi

cfg="$out/openssl.cnf"
cat >"$cfg" <<'EOF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = MFB test CA
[leaf_dn]
CN = 127.0.0.1
[leaf_ext]
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = IP:127.0.0.1, DNS:localhost
[ca_ext]
basicConstraints = critical, CA:TRUE, pathlen:0
keyUsage = critical, keyCertSign, cRLSign
EOF

# The CA.
openssl req -x509 -newkey rsa:2048 -noenc \
  -keyout "$out/ca-key.pem" -out "$out/ca.pem" \
  -days 3650 -config "$cfg" -extensions ca_ext >/dev/null 2>&1

# The server leaf, signed by that CA.
openssl req -newkey rsa:2048 -noenc \
  -keyout "$out/server-key-pkcs8.pem" -out "$out/server.csr" \
  -config "$cfg" -reqexts leaf_ext \
  -subj "/CN=127.0.0.1" >/dev/null 2>&1
openssl x509 -req -in "$out/server.csr" \
  -CA "$out/ca.pem" -CAkey "$out/ca-key.pem" -CAcreateserial \
  -out "$out/server.pem" -days 3650 \
  -extfile "$cfg" -extensions leaf_ext >/dev/null 2>&1

# `openssl req` emits PKCS#8; convert to the traditional PKCS#1 encoding macOS
# accepts, and keep both.
openssl rsa -in "$out/server-key-pkcs8.pem" -traditional \
  -out "$out/server-key.pem" >/dev/null 2>&1

cat "$out/server.pem" "$out/ca.pem" >"$out/chain.pem"
rm -f "$out/server.csr" "$out/.srl" "$out/ca.srl"

# Fail loudly rather than leaving a half-made identity behind.
for f in ca.pem ca-key.pem server.pem server-key.pem server-key-pkcs8.pem chain.pem; do
  [ -s "$out/$f" ] || { echo "FAIL: $out/$f was not generated" >&2; exit 1; }
done
head -1 "$out/server-key.pem" | grep -q 'BEGIN RSA PRIVATE KEY' \
  || { echo "FAIL: server-key.pem is not PKCS#1" >&2; exit 1; }
head -1 "$out/server-key-pkcs8.pem" | grep -q 'BEGIN PRIVATE KEY' \
  || { echo "FAIL: server-key-pkcs8.pem is not PKCS#8" >&2; exit 1; }
openssl verify -CAfile "$out/ca.pem" "$out/server.pem" >/dev/null 2>&1 \
  || { echo "FAIL: the leaf does not verify against the generated CA" >&2; exit 1; }

echo "generated test TLS identity in $out (CN=127.0.0.1, SAN IP:127.0.0.1)"
