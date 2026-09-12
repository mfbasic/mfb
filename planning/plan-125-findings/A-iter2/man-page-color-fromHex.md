NO FINDINGS for man-page:color/fromHex

I ran all three rendered examples; they printed `#ff00aaff`, then two `#3366ccff` lines, then `#3366ccff`, `rejected #12345: TRUE`, and `#000000ff`. My boundary probe printed opaque results for 3/6 digits, preserved alpha for 4/8 digits, and `ErrInvalidFormat` for empty input, `#`, doubled `#`, whitespace, non-ASCII input, and unsupported lengths.