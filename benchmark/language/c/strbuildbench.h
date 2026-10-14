#ifndef STRBUILDBENCH_H
#define STRBUILDBENCH_H
/* test_string_unibig belongs to the `string` group (called right after
 * run_string_group so the string rows stay contiguous). Its Unicode
 * grapheme/normalize counts are an APPROXIMATION (documented, like the
 * `string unicode` row) and are not required to match mfb bit-for-bit. */
void test_string_unibig(void);
void run_strbuild_group(void);
#endif
