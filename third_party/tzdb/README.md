# third_party/tzdb

The IANA Time Zone Database, vendored unmodified. `packages/timezones` answers every
named-zone question from these bytes and never from the host's zone database
(plan-135).

## Release

`2026d/` holds IANA tzdb **2026d**, byte-for-byte as published:

| File | Source |
|---|---|
| `tzdata2026d.tar.gz` | <https://data.iana.org/time-zones/releases/tzdata2026d.tar.gz> |
| `tzdata2026d.tar.gz.asc` | <https://data.iana.org/time-zones/releases/tzdata2026d.tar.gz.asc> |
| `tzcode2026d.tar.gz` | <https://data.iana.org/time-zones/releases/tzcode2026d.tar.gz> |
| `tzcode2026d.tar.gz.asc` | <https://data.iana.org/time-zones/releases/tzcode2026d.tar.gz.asc> |
| `SHA256SUMS` | `shasum -a 256 tzdata2026d.tar.gz tzcode2026d.tar.gz` |

`tools/tzdb/gen_timezones_data.py` checks both tarballs against `SHA256SUMS` before it
reads them. Then it compiles them with the release's own `zic`.

## Verification (2026-09-13)

```
$ cd third_party/tzdb/2026d && shasum -a 256 -c SHA256SUMS
tzdata2026d.tar.gz: OK
tzcode2026d.tar.gz: OK
```

The signing key is Paul Eggert's, the tz coordinator. It is fetched over HTTPS
because the HKP keyserver port was unreachable from the verifying host:

```
$ curl -sSfL https://keys.openpgp.org/vks/v1/by-fingerprint/7E3792A9D8ACF7D633BC1588ED97E90E62AA7E34 | gpg --import
$ gpg --verify tzdata2026d.tar.gz.asc tzdata2026d.tar.gz
gpg: Signature made Fri Sep 11 12:24:25 2026 HST
gpg:                using RSA key 7E3792A9D8ACF7D633BC1588ED97E90E62AA7E34
gpg: Good signature from "Paul Eggert <eggert@cs.ucla.edu>" [unknown]
$ gpg --verify tzcode2026d.tar.gz.asc tzcode2026d.tar.gz
gpg: Signature made Fri Sep 11 12:24:24 2026 HST
gpg:                using RSA key 7E3792A9D8ACF7D633BC1588ED97E90E62AA7E34
gpg: Good signature from "Paul Eggert <eggert@cs.ucla.edu>" [unknown]
```

Fingerprint: `7E37 92A9 D8AC F7D6 33BC 1588 ED97 E90E 62AA 7E34`. Its expiry is
2031-07-24.

Fetch the key from keys.openpgp.org, not keyserver.ubuntu.com. The
keyserver.ubuntu.com copy was stale on 2026-09-13: it still listed the old
2026-09-01 expiry, so gpg printed `[expired]` next to the good signatures.
`[unknown]` only means that no local key vouches for this one. Compare the
fingerprint above instead.

## Licence

The release's `LICENSE` file:

> Unless specified below, all files in the tz code and data (including
> this LICENSE file) are in the public domain.
>
> If the files date.c, newstrftime.3, and strftime.c are present, they
> contain material derived from BSD and use the BSD 3-clause license.

The generator compiles only `zic`. It uses none of the BSD-licensed files, and none of
their material reaches `packages/timezones`.

## Updating

Follow the update procedure in [`tools/tzdb/README.md`](../../tools/tzdb/README.md).
