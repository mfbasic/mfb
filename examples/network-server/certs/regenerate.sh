#!/bin/sh
# Regenerate the example's self-signed TLS identity.
#
# The pair is DELIBERATELY self-signed and untrusted: examples/network-client
# reaches it with `allowSelfSigned := TRUE`, and that is the behaviour the pair
# exists to demonstrate. Do not replace it with a publicly-trusted certificate.
#
# Two options are load-bearing and easy to lose:
#
#   -days 397                          Apple refuses a TLS server certificate
#   -addext extendedKeyUsage=serverAuth whose validity window exceeds ~398 days
#                                      or which lacks serverAuth, as "not
#                                      standards compliant" — regardless of what
#                                      the client trusts. A longer-lived pair
#                                      works on Linux and Windows and fails on
#                                      macOS, which is a confusing way to find
#                                      out. See .ai/net-tls.md.
#
# Because of that 397-day ceiling this pair EXPIRES. Re-run this script from
# examples/network-server/certs when the TLS attempt starts reporting a failure
# on every platform.
set -eu
cd "$(dirname "$0")"
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout key.pem -out cert.pem \
  -days 397 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "extendedKeyUsage=serverAuth"
openssl x509 -in cert.pem -noout -subject -dates -ext subjectAltName,extendedKeyUsage
