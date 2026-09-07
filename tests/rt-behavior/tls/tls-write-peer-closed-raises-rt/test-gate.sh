#!/usr/bin/env bash
# Environment gate for tls-write-peer-closed-raises-rt (consumed by
# scripts/test-accept.sh).
#
# This fixture needs no network, but it does need the `openssl` CLI on PATH, for
# two things it cannot do itself: mint a throwaway server certificate/key pair,
# and be the FOREIGN TLS peer (`openssl s_client`) that the MFBASIC server is
# proven against. Using `tls::connect` as the peer instead would make this a
# proof that our client and our server agree with each other, which cannot tell a
# working TLS implementation from two matching bugs — the same reasoning
# `scripts/check-tls-loopback.sh` and `tests/rt_tls_connect_allow_self_signed.rs`
# record.
#
# Committing a certificate pair instead would remove the dependency and add a
# worse one: every certificate expires, and macOS additionally refuses a server
# certificate whose validity window exceeds ~398 days (`.ai/net-tls.md`), so a
# static pair turns this fixture red on a DATE rather than on a regression.
#
# The plaintext half of the same contract needs neither —
# `rt-behavior/tcp/tcp-write-peer-closed-raises-rt` runs with no dependency at
# all.
set -u

if command -v openssl >/dev/null 2>&1; then
  exit 0
fi

echo "openssl not found on PATH (this fixture mints a throwaway TLS identity and uses openssl s_client as the foreign peer)"
exit 1
