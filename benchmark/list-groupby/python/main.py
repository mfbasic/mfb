"""collections::groupBy stress: group a 2000-element list into 100 buckets
(i mod 100), appending each item to its bucket's list. Prints the number of
groups."""


def main():
    groups = {}
    for i in range(2000):
        k = i % 100
        groups.setdefault(k, []).append(i)
    print("groups: " + str(len(groups)))
    return 0


if __name__ == "__main__":
    main()
