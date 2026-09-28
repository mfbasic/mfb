# sorts

25,000 dots on a spinning sphere, each coloured by where its slot sits on the
sphere, put in order by eighteen sorting algorithms one after another. Each
sort runs a few steps every frame, and every element it moves sends that dot
flying toward its new slot, so you watch the sphere assemble the way that
algorithm works. See the header comment in [`src/main.mfb`](src/main.mfb:1-42)
for the full picture.

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/sorts
```

- macOS — `open examples/sorts/build/sorts.app`
- Linux — `./examples/sorts/build/sorts.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/sorts`,
  then run `examples/sorts/build/sorts.exe`

It opens fullscreen. **Esc** quits; there are no other controls.

## What you see

The run starts with every dot at the centre. Then, for each sort in turn and
round again forever:

1. the dots fly out to random points over three seconds, and the list is
   shuffled;
2. they hold still for two seconds;
3. the sort runs;
4. the finished sphere stays up for four seconds.

Top left, while a sort is on screen: its name, **Sorted** — the share of dots
sitting in their own slot — and **Compares**, the comparisons it has made.
Top right: presents and frames drawn per second. Along the bottom, dimmed, a
table of every sort's comparison count from the last time it finished.

## The sorts

In the order they run, one file each under `src/`:

| Sort | File | Steps per second |
| --- | --- | --- |
| Bubble | `bubblesort.mfb` | 10,000,000 |
| Selection | `selectionsort.mfb` | 10,000,000 |
| Cocktail shaker | `cocktailsort.mfb` | 10,000,000 |
| Odd-even transposition | `oddevensort.mfb` | 10,000,000 |
| Gnome | `gnomesort.mfb` | 10,000,000 |
| Insertion | `insertionsort.mfb` | 5,000,000 |
| Binary insertion | `binaryinsertionsort.mfb` | 10,000 |
| Cycle | `cyclesort.mfb` | 20,000,000 |
| Comb | `combsort.mfb` | 30,000 |
| Shell (Ciura gaps) | `shellsort.mfb` | 30,000 |
| Circle | `circlesort.mfb` | 100,000 |
| Batcher merge-exchange | `batchersort.mfb` | 40,000 |
| Heap | `heapsort.mfb` | 25,000 |
| Merge (bottom-up) | `mergesort.mfb` | 15,000 |
| Quick (Lomuto) | `quicksort.mfb` | 15,000 |
| Dual-pivot quick | `dualpivotsort.mfb` | 15,000 |
| Bucket (500 buckets) | `bucketsort.mfb` | 12,000 |
| Radix (LSD, base 10) | `radixsort.mfb` | 4,000 |

Each rate is set so its sort takes roughly half a minute, which means the
quadratic sorts run hundreds of times faster than the others. **Compare the
sorts by their comparison counts, not by how long they take.** A step is one
comparison, except for radix, which never compares two elements (its count
shows N/A) and spends its steps on writes, and bucket, whose first stage deals
the dots into buckets one write at a time.

Every sort has the same four entry points — `<name>Start`,
`<name>Steps(budget)`, `<name>Done` and `<name>Compares` — and touches the list
only through `orderAt`, `setOrder` and `swapOrder` in `main.mfb`, so adding a
sort means one new file and one line in each dispatch function in `main.mfb`.

No dependencies beyond `app`, `canvas`, `collections`, `color`, `datetime`,
`math`, `strings`, and the other standard packages the program imports.
