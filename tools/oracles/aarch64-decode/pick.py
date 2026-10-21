import sys, re
per = int(sys.argv[3]) if len(sys.argv) > 3 else 1
existing = sys.argv[4] if len(sys.argv) > 4 else None
import os
LS = int(os.environ.get('LONG_SHIFT', '24'))
LM = int(os.environ.get('LONG_MASK', '0'))
COND = r'\b(eq|ne|cs|hs|cc|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le|al|nv)\b'

def key_of(word, text):
    if text.startswith('.long'):
        return ('.long', (int(word, 16) >> LS, (int(word, 16) >> 10) & LM))
    mn, _, ops = text.partition(' ')
    mn = re.sub(r'^(b|bc)\.\w+$', r'\1.C', mn)
    ops = re.sub(r'\bS\d+_\d+_C\d+_C\d+_\d+\b', 'SGEN', ops.strip())
    ops = re.sub(r'(?<![A-Za-z_0-9.])-?\d+(?![A-Za-z_0-9])', 'N', ops)
    shape = re.sub(r'#?-?0x[0-9a-f]+|#-?\d+|(?<![A-Za-z_0-9])([xwvqsdhbzc])\d+(?![A-Za-z_0-9])', lambda m: (m.group(1) or '') + 'N', ops)
    shape = re.sub(COND, 'C', shape)
    return (mn, shape)

seen = {}
if existing:
    for line in open(existing):
        if line.startswith('#') or not line.strip():
            continue
        pc, word, text = line.rstrip('\n').split('\t', 2)
        seen[key_of(word, text)] = per
bad = set()
if os.environ.get('BAD'):
    for l in open(os.environ['BAD']):
        bad.add(l[2:10])
out = []
for line in open(sys.argv[1]):
    pc, word, text = line.rstrip('\n').split('\t', 2)
    if word in bad:
        continue
    if text.startswith('udf'):
        continue
    key = key_of(word, text)
    n = seen.get(key, 0)
    if n >= per:
        continue
    seen[key] = n + 1
    out.append(line)
open(sys.argv[2], 'w').write(''.join(out))
print(len(out))
