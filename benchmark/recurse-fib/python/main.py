"""Naive recursive Fibonacci — exercises pure call/return overhead."""


def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def main():
    result = fib(35)
    print("fib(35): " + str(result))
    return 0


if __name__ == "__main__":
    main()
