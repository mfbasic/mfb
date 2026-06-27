"""Builds a 1000-item dict, then looks each key up to verify its content."""


def main():
    m = {}
    for i in range(1000):
        m[str(i)] = i

    total = 0
    for i in range(1000):
        total += m[str(i)]

    print("count: " + str(len(m)) + " sum: " + str(total))
    return 0


if __name__ == "__main__":
    main()
