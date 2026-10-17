# third_party/unicode

Unicode Character Database files, vendored unmodified, all from **Unicode 16.0.0** —
the version the compiler's pinned tables use (`mfb spec unicode tables-and-algorithms`).
The generators under `tools/unicode-tables/` read them; nothing reads them at run time.

| File | Source | Read by |
|---|---|---|
| `Scripts-16.0.0.txt` | <https://www.unicode.org/Public/16.0.0/ucd/Scripts.txt> | `gen_regex_scripts.py`, `gen_unicode_script_table.py` |
| `LineBreak-16.0.0.txt` | <https://www.unicode.org/Public/16.0.0/ucd/LineBreak.txt> | `gen_linebreak_table.py` |
| `EastAsianWidth-16.0.0.txt` | <https://www.unicode.org/Public/16.0.0/ucd/EastAsianWidth.txt> | `gen_linebreak_table.py` (UAX #14 LB30's F/W/H exclusion) |
| `emoji-data-16.0.0.txt` | <https://www.unicode.org/Public/16.0.0/ucd/emoji/emoji-data.txt> | `gen_linebreak_table.py` (Extended_Pictographic, LB30b) |
| `LineBreakTest-16.0.0.txt` | <https://www.unicode.org/Public/16.0.0/ucd/auxiliary/LineBreakTest.txt> | `tests/runtime/rt_strings_line_breaks.rs` (the conformance oracle) |
| `LICENSE.txt` | <https://www.unicode.org/license.txt> | — |

The files are © Unicode, Inc. and are distributed under the Unicode License v3, which
`LICENSE.txt` reproduces.

SHA-256 of the files as downloaded (2026-10-17):

```
43adc76c0686a42cb370764eb8cfe2b2a45b10b855e5572a2db4a0eecce15d5b  EastAsianWidth-16.0.0.txt
e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96  LICENSE.txt
e97e4259d0d20fab150b9c7b4b28abfae5cd78ca97e7f4ac6ed20d685d5f4a7c  LineBreak-16.0.0.txt
910759a611a479f37df4f535d2e64d7589be3c5ac5f7491cf1fb4fa4cdb211e9  LineBreakTest-16.0.0.txt
9e88f0a677df47311106340be8ede2ecdacd9c1c931831218d2be6d5508e0039  Scripts-16.0.0.txt
f1365a5173eee18e1f98b240cdc492e84a25f1ce7e0c9d1094eb29c41a22696a  emoji-data-16.0.0.txt
```
