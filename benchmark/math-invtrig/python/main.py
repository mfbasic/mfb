"""Inverse-trig kernel stress test — Python reference (see ../mfb/src/main.mfb)."""
from math import asin, acos, atan


def main() -> None:
    acc = 0.0
    for _rep in range(2000):
        t = -0.999
        for _i in range(1000):
            acc += asin(t) + acos(t) + atan(t)
            t += 0.001998
    print(f"invtrig: {acc:.6f}")


if __name__ == "__main__":
    main()
