// terrain_dump: run the terrain half of the oracle's digDungeon() -- everything
// before addMachines() -- for one level seed and depth, and print the working
// grid or the map after each step, in the format `brogue.out dig` prints.
//
// Architect.c is #included rather than linked so the driver can call its
// static stage functions in digDungeon()'s own order. The stage sequence below
// must stay a line-for-line copy of digDungeon() up to addMachines().
//
// Usage: terrain_dump <levelSeed> <depth>

#include <stdio.h>
#include <stdlib.h>
#include "Architect.c"
#include "GlobalsBrogue.h"

static void dumpGrid(const char *stage, short **grid) {
    printf("stage %s grid rng=%lu\n", stage, randomNumbersGenerated);
    for (int j = 0; j < DROWS; j++) {
        for (int i = 0; i < DCOLS; i++) {
            printf(i ? " %d" : "%d", grid[i][j]);
        }
        printf("\n");
    }
}

static void dumpMap(const char *stage) {
    static const char *layerNames[NUMBER_TERRAIN_LAYERS] = {"dungeon", "liquid", "gas", "surface"};
    printf("stage %s map rng=%lu\n", stage, randomNumbersGenerated);
    for (int layer = 0; layer < NUMBER_TERRAIN_LAYERS; layer++) {
        printf("layer %s\n", layerNames[layer]);
        for (int j = 0; j < DROWS; j++) {
            for (int i = 0; i < DCOLS; i++) {
                printf(i ? " %d" : "%d", pmap[i][j].layers[layer]);
            }
            printf("\n");
        }
    }
    printf("flags\n");
    for (int j = 0; j < DROWS; j++) {
        for (int i = 0; i < DCOLS; i++) {
            printf(i ? " %lu" : "%lu", pmap[i][j].flags);
        }
        printf("\n");
    }
    printf("volume\n");
    for (int j = 0; j < DROWS; j++) {
        for (int i = 0; i < DCOLS; i++) {
            printf(i ? " %d" : "%d", pmap[i][j].volume);
        }
        printf("\n");
    }
}

int main(int argc, char *argv[]) {
    if (argc != 3) {
        fprintf(stderr, "usage: terrain_dump <levelSeed> <depth>\n");
        return 2;
    }
    uint64_t levelSeed = strtoull(argv[1], NULL, 10);
    short depth = (short) strtol(argv[2], NULL, 10);

    initializeGameVariantBrogue();
    rogue.depthLevel = depth;
    rogue.RNG = RNG_SUBSTANTIVE;
    randomNumbersGenerated = 0;
    seedRandomGenerator(levelSeed);

    // digDungeon(), up to addMachines().
    short i, j;
    short **grid;

    rogue.machineNumber = 0;
    topBlobMinX = topBlobMinY = blobWidth = blobHeight = 0;

    clearLevel();

    grid = allocGrid();
    carveDungeon(grid);
    dumpGrid("carve", grid);
    addLoops(grid, 20);
    dumpGrid("loops", grid);
    for (i=0; i<DCOLS; i++) {
        for (j=0; j<DROWS; j++) {
            if (grid[i][j] == 1) {
                pmap[i][j].layers[DUNGEON] = FLOOR;
            } else if (grid[i][j] == 2) {
                pmap[i][j].layers[DUNGEON] = (rand_percent(60) && rogue.depthLevel < gameConst->deepestLevel ? DOOR : FLOOR);
            }
        }
    }
    freeGrid(grid);

    finishWalls(false);
    dumpMap("walls");

    short **lakeMap = allocGrid();
    designLakes(lakeMap);
    dumpGrid("lakes", lakeMap);
    fillLakes(lakeMap);
    freeGrid(lakeMap);
    dumpMap("filled");

    runAutogenerators(false);
    dumpMap("autogen");

    removeDiagonalOpenings();
    dumpMap("diagonals");
    return 0;
}
