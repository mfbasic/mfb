"""Copies a 1000-item string list 1000 times, then a 1000-item record list."""


def copy_strings(xs):
    return list(xs)


def copy_recs(xs):
    return list(xs)


def main():
    strs = []
    for i in range(1000):
        strs.append(str(i))
    acc = 0
    for i in range(1000):
        c = copy_strings(strs)
        acc = acc + len(c)

    recs = []
    for i in range(1000):
        recs.append((i, str(i)))
    acc_recs = 0
    for i in range(1000):
        c = copy_recs(recs)
        acc_recs = acc_recs + len(c)

    print("strings: " + str(acc) + " recs: " + str(acc_recs))
    return 0


if __name__ == "__main__":
    main()
