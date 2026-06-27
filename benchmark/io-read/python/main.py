"""Reads standard input line by line until EOF, counting lines and total bytes
(excluding the line terminators)."""

import sys


def main():
    lines = 0
    nbytes = 0
    for line in sys.stdin:
        lines += 1
        nbytes += len(line.rstrip("\n").rstrip("\r"))
    print("lines: " + str(lines) + " bytes: " + str(nbytes))
    return 0


if __name__ == "__main__":
    main()
