#!/bin/sh
# fs error-matrix fixtures (bug-694, bug-701): run from the directory the probe runs in.
chmod -R u+rwx fx 2>/dev/null
rm -rf fx
mkdir fx
cd fx || exit 1
printf 'secret' > noread.txt
chmod 000 noread.txt
mkdir -p full/sub
printf 'x' > full/f
ln -s loopb loopa
ln -s loopa loopb
ln -s /etc/hosts link.txt
mkdir ro
printf 'x' > ro/inside.txt
mkdir ro/emptydir
chmod 555 ro
mkdir -p nox/child
chmod 000 nox
printf '\377\376' > bad.txt
printf 'a\nb' > lines.txt
printf 'file' > plain.txt
