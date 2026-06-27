"""Builds a list of 50 random integers, then copies and sorts it SORTS times."""

import random


def main():
    sorts = 1
    base = [random.randint(0, 1000000) for _ in range(50)]

    checksum = 0
    last_sorted = base
    for i in range(sorts):
        last_sorted = sorted(base)
        checksum = checksum + last_sorted[0]

    ok = 1
    for i in range(1, len(last_sorted)):
        if last_sorted[i] < last_sorted[i - 1]:
            ok = 0
    print("count: " + str(len(base)) + " sorted: " + str(ok))
    return 0


if __name__ == "__main__":
    main()
