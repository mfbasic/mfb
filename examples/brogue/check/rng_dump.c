// rng_dump: print Brogue CE's RNG stream for one seed, from the oracle's own
// Math.c, in the exact format `brogue.out rng <seed> <count>` prints.
//
// The call script mixes every public RNG entry point and both streams, so a
// port that gets any of them wrong -- the 32-bit wraparound in ranval, the
// rejection loop in range, the clump split, the 64-bit seed path -- diverges
// on a numbered line.

#include <stdio.h>
#include <stdlib.h>
#include "Rogue.h"
#include "GlobalsBase.h"

int main(int argc, char *argv[]) {
    if (argc != 3) {
        fprintf(stderr, "usage: rng_dump <seed> <count>\n");
        return 2;
    }
    uint64_t seed = strtoull(argv[1], NULL, 10);
    long count = strtol(argv[2], NULL, 10);

    rogue.RNG = RNG_SUBSTANTIVE;
    randomNumbersGenerated = 0;
    seedRandomGenerator(seed);

    for (long i = 0; i < count; i++) {
        rogue.RNG = (i % 7 == 6) ? RNG_COSMETIC : RNG_SUBSTANTIVE;
        switch (i % 5) {
            case 0:
                printf("%ld range %ld\n", i, rand_range(0, 9999));
                break;
            case 1:
                printf("%ld range %ld\n", i, rand_range(-50, 50 + i % 13));
                break;
            case 2:
                printf("%ld clump %d\n", i, randClumpedRange(1, 20 + i % 11, 1 + i % 4));
                break;
            case 3:
                printf("%ld percent %d\n", i, rand_percent(i % 101) ? 1 : 0);
                break;
            case 4: {
                uint64_t v = rand_64bits();
                printf("%ld bits64 %llu %llu\n", i,
                       (unsigned long long) (v >> 32),
                       (unsigned long long) (v & 0xFFFFFFFFull));
                break;
            }
        }
    }
    rogue.RNG = RNG_SUBSTANTIVE;
    short list[10];
    fillSequentialList(list, 10);
    shuffleList(list, 10);
    printf("shuffle");
    for (int i = 0; i < 10; i++) {
        printf(" %d", list[i]);
    }
    printf("\n");
    printf("generated %lu\n", randomNumbersGenerated);
    return 0;
}
