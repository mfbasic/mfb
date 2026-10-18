# bug-1107: a program that calls `crypto::seal` and `crypto::open` ends with 4,592 bytes live

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: none yet (a `--debug` run whose report shows `arena.0.live_bytes 0`)

Found by bs24-agent-f while fixing bug-863, reproduced by the bug-staging-24 coordinator,
and placed in a later phase.

**The single correct behavior a fix produces:** the program below reports
`arena.0.live_bytes 0` at exit, as a program without the two calls does.

## Reproduction (main `c60f81037`)

```
IMPORT crypto
IMPORT io

FUNC main() AS Nothing
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
  LET box AS crypto::Sealed = crypto::seal(crypto::SymmetricCipher.AES256GCM, key, nonce, "hello")
  LET back AS List OF Byte = crypto::open(crypto::SymmetricCipher.AES256GCM, key, nonce, box.ciphertext, box.tag)
  io::print(toString(len(back)))
END FUNC
```

`mfb build -q --debug . && ./build/<name>.out`: prints `5`, then the report:
`arena.0.alloc_calls 434`, `arena.0.free_calls 421`, `arena.0.live_bytes 4592`,
`arena.0.peak_live_bytes 6400`. A program that only prints: 0 / 0 / `live_bytes 0`.
Which of the 13 unfreed blocks belong to `seal`, `open` or `randomBytes` is not yet traced.
