"""Approximates pi via the Leibniz series.

pi = 4 * sum_{k=0..N-1} (-1)^k / (2k+1), accumulated with an alternating sign.
"""


def main():
    total = 0.0
    sign = 1.0
    for k in range(1000000):
        denom = float(2 * k + 1)
        total = total + sign / denom
        sign = sign * -1.0
    pi = 4.0 * total
    print("pi: %.5f" % pi)
    return 0


if __name__ == "__main__":
    main()
