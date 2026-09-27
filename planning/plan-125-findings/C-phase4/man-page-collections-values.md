NO FINDINGS for man-page:collections/values

I compiled and ran all three examples: they printed `1`, `121`, and `36` then `85`. The edge probe printed `0`, `7`, `7`, `9`, `first`, `second`, `third`, confirming empty-map behavior, duplicates, insertion order, and keys/values alignment. An inline `TRAP` probe emitted `TYPE_INLINE_TRAP_DEAD_HANDLER` and printed `1`.