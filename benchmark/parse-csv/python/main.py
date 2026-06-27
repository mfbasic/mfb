"""Reads a pre-generated 2000-row, 3-column integer CSV, parses it, then sums
every cell."""

import csv


def main():
    with open("/tmp/mfb-bench-parse-csv.csv") as f:
        grid = list(csv.reader(f))

    total = 0
    for row in grid:
        for cell in row:
            total += int(cell)

    print("rows: " + str(len(grid)) + " sum: " + str(total))
    return 0


if __name__ == "__main__":
    main()
