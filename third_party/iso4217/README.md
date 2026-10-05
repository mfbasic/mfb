# third_party/iso4217

The ISO 4217 currency code list, vendored unmodified. `packages/currency` decides
which three-letter codes are real currencies, and how many minor units each one has,
from these bytes alone.

## Release

`2026-09-17/` holds ISO 4217 Amendment List One as published on **2026-09-17** (the
file's `Pblshd` attribute), byte-for-byte as downloaded from the maintenance agency,
SIX Financial Information:

| File | Source |
|---|---|
| `list-one.xml` | <https://www.six-group.com/dam/download/financial-information/data-center/iso-currrency/lists/list-one.xml> |
| `SHA256SUMS` | `shasum -a 256 list-one.xml` |

The misspelled `iso-currrency` path segment is SIX's own spelling.

`tools/iso4217/gen_currency_data.py` checks `list-one.xml` against `SHA256SUMS` before it
reads it.

## Verification (2026-10-04)

SIX publishes no signature for the list, so the download is checked only by its
transport (HTTPS) and the checksum recorded at vendoring time:

```
$ cd third_party/iso4217/2026-09-17 && shasum -a 256 -c SHA256SUMS
list-one.xml: OK
```

Updating to a newer list is described in `tools/iso4217/README.md`.
