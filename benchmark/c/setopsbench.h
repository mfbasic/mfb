#ifndef SETOPSBENCH_H
#define SETOPSBENCH_H
/* set group (plan-65 Theme 3): the `Set OF T` collection type (plan-63). Two
 * rows mirroring benchmark/mfb/src/setops.mfb: `build` (grow-by-add hash-probe
 * plus a membership sweep) and `ops` (the full set-algebra surface — union /
 * intersection / difference / symmetricDifference / isSubset / isSuperset /
 * isDisjoint / toList / toSet / remove). The C peer is an open-addressing
 * integer hash set; its checksums (20000 and 6006) match the mfb and Python
 * columns bit-for-bit. See setopsbench.c. */
void run_setops_group(void);
#endif
