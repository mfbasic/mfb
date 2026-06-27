/* Reads standard input line by line until EOF, counting lines and total bytes
 * (excluding the line terminators). The benchmark input has no CR bytes. */
#include <stdio.h>

int main(void) {
  long lines = 0, bytes = 0, linelen = 0;
  int in_line = 0, c;

  while ((c = getchar()) != EOF) {
    if (c == '\n') {
      lines = lines + 1;
      bytes = bytes + linelen;
      linelen = 0;
      in_line = 0;
    } else if (c != '\r') {
      linelen = linelen + 1;
      in_line = 1;
    }
  }
  if (in_line) { /* final line without a trailing newline */
    lines = lines + 1;
    bytes = bytes + linelen;
  }

  printf("lines: %ld bytes: %ld\n", lines, bytes);
  return 0;
}
