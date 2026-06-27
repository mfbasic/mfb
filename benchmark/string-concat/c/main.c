/* Builds a string with 1000 concatenations, one character at a time. */
#include <stdio.h>
#include <stdlib.h>

int main(void) {
  char *s = NULL;
  int len = 0, cap = 0;
  for (int i = 0; i < 1000; i++) {
    if (len + 1 >= cap) {
      cap = cap ? cap * 2 : 2;
      s = realloc(s, cap);
    }
    s[len] = 'x';
    len = len + 1;
  }
  s[len] = '\0';

  printf("len: %d\n", len);
  return 0;
}
