"""collections::distinct stress: build a 5000-element list with heavy
duplication (i mod 1000), then keep first occurrences in order. The naive
O(n^2) "scan the kept list" form mirrors mfb's contains()-in-a-loop
implementation. Prints the number of distinct values."""


def main():
    nums = [i % 1000 for i in range(5000)]
    unique = []
    for n in nums:
        if n not in unique:
            unique.append(n)
    print("count: " + str(len(unique)))
    return 0


if __name__ == "__main__":
    main()
