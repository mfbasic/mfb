"""Exp/log/power kernel stress test — Python reference (see ../mfb/src/main.mfb)."""
from math import exp, log, log10, pow


def main() -> None:
    acc = 0.0
    for _rep in range(2000):
        v = 0.001
        for _i in range(1000):
            acc += exp(v * 0.1) + log(v) + log10(v) + pow(v, 1.5)
            v += 0.005
    print(f"explog: {acc:.6f}")


if __name__ == "__main__":
    main()
