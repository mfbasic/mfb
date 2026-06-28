"""Forward-trig kernel stress test — Python reference (see ../mfb/src/main.mfb)."""
from math import sin, cos, tan, atan2


def main() -> None:
    acc = 0.0
    for _rep in range(2000):
        x = 0.001
        for _i in range(1000):
            acc += sin(x) + cos(x) + tan(x) + atan2(x, 1.0 + x)
            x += 0.0015
    print(f"trig: {acc:.6f}")


if __name__ == "__main__":
    main()
