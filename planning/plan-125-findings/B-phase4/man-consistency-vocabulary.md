### 1. Stream text unit is called “character” instead of “Unicode scalar”
DIMENSION: vocabulary  
SCOPE:     tcp, tls, udp; strings and types establish the competing term  
CLAIM:     tcp says a stream split “need not be a character boundary”; tls says it “can split a multi-byte character in half”; udp says it cannot split “a character in half.” The strings overview instead establishes that String operations use “Unicode scalar values,” not bytes or graphemes.  
VERDICT:   “Unicode scalar” should win. It is the documented String unit, and `io::readChar` also explicitly returns one Unicode scalar value. “Character” is ambiguous with grapheme clusters.  
EVIDENCE:  `grep -ci 'Unicode scalar' condensed.txt` printed `23`; `grep -ci 'character boundary' condensed.txt` printed `2`; `grep -ci 'character in half' condensed.txt` printed `1`. `rg -ni 'Unicode scalar|character boundary|character in half' condensed.txt` located the minority form in tcp, tls, and udp.  
SUGGESTED: Standardize the stream wording on “Unicode-scalar boundary” and “split a multi-byte Unicode scalar,” retaining “byte” for the opaque transport payload.