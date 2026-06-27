/* Stresses copy-on-update of records: build a 100-element array of structs,
 * then run 10 passes incrementing the n field of every record. Prints the
 * checksum (sum of all n fields). */
#include <stdio.h>

struct Rec {
  int n;
  char label[16];
};

int main(void) {
  struct Rec recs[100];
  for (int i = 0; i < 100; i++) {
    recs[i].n = i;
    snprintf(recs[i].label, sizeof(recs[i].label), "p%d", i);
  }

  for (int pass = 0; pass < 10; pass++) {
    for (int j = 0; j < 100; j++) {
      recs[j].n = recs[j].n + 1;
    }
  }

  long checksum = 0;
  for (int j = 0; j < 100; j++) {
    checksum += recs[j].n;
  }

  printf("checksum: %ld\n", checksum);
  return 0;
}
