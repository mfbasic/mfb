#ifndef MATHPIPE_H
#define MATHPIPE_H
/* matmul extends the `float` group; dft/stats form the `mathpipe` group.
 * (finance is mfb-only — Money has no C peer — so there is no C row for it.) */
void test_matmul(void);
void run_mathpipe_group(void);
#endif
