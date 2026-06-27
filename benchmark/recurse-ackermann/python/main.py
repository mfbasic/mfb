"""Ackermann function — deeply nested recursive call/return overhead."""

import sys

sys.setrecursionlimit(1000000)


def ack(m, n):
    if m == 0:
        return n + 1
    if n == 0:
        return ack(m - 1, 1)
    return ack(m - 1, ack(m, n - 1))


def main():
    result = ack(3, 7)
    print("ack(3,7): " + str(result))
    return 0


if __name__ == "__main__":
    main()
