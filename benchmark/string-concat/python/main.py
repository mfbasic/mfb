"""Builds a string with 1000 concatenations, one character at a time."""


def main():
    s = ""
    for i in range(1000):
        s = s + "x"
    print("len: " + str(len(s)))
    return 0


if __name__ == "__main__":
    main()
