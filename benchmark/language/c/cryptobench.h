#ifndef CRYPTOBENCH_H
#define CRYPTOBENCH_H
/* crypto group (plan-65 Theme 1): SHA-256/512, HMAC-SHA-256, PBKDF2, constant-
 * time compare, hash churn. Ed25519 is recorded as a `--` placeholder (mfb+python
 * only). See cryptobench.c. */
void run_crypto_group(void);
#endif
