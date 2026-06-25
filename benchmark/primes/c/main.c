/* Calculates and prints the first 1000 prime numbers. */
#include <stdio.h>

/* Returns 1 if n is prime by trial division up to sqrt(n). */
static int is_prime(int n) {
  if (n < 2) {
    return 0;
  }
  int i = 2;
  while (i * i <= n) {
    if (n % i == 0) {
      return 0;
    }
    i = i + 1;
  }
  return 1;
}

/* Fills `primes` with the first `count` primes. */
static void first_primes(int *primes, int count) {
  int found = 0;
  int candidate = 2;
  while (found < count) {
    if (is_prime(candidate)) {
      primes[found] = candidate;
      found = found + 1;
    }
    candidate = candidate + 1;
  }
}

int main(void) {
  int primes[1000];
  first_primes(primes, 1000);
  for (int i = 0; i < 1000; i++) {
    printf("%d\n", primes[i]);
  }
  return 0;
}
