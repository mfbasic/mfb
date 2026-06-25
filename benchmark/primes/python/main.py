"""Calculates and prints the first 1000 prime numbers."""


def is_prime(n):
    """Returns True if n is prime by trial division up to sqrt(n)."""
    if n < 2:
        return False
    i = 2
    while i * i <= n:
        if n % i == 0:
            return False
        i = i + 1
    return True


def first_primes(count):
    """Builds a list of the first `count` primes."""
    primes = []
    candidate = 2
    while len(primes) < count:
        if is_prime(candidate):
            primes = primes + [candidate]
        candidate = candidate + 1
    return primes


def main():
    primes = first_primes(1000)
    for i in range(len(primes)):
        print(str(primes[i]))
    return 0


if __name__ == "__main__":
    main()
