/* GROUP: interp — the C peer for interp.mfb (a small stack bytecode VM).
 *
 * mfb fetches an Instr record (ENUM opcode + Integer operand) from a
 * `List OF Instr`, dispatches with MATCH, and keeps its operand stack in a
 * `MUT List OF Integer` via collections::get/set. The C analogue is a switch
 * over an enum with an int64 array stack. The same two programs are assembled
 * once (untimed), instruction-for-instruction identical to mfb:
 *   vm       iterative Fibonacci mod 1000000007, n = 100, repeated 1500 times
 *   vm_loop  a 400 x 400 nested counting loop, acc = (acc + i*j + 1) mod 1000003
 * bench_opaque(prog) before each run stops -O2 hoisting the loop-invariant
 * interpretation out of the timed loop.
 * Checksums (vm=993890033, vm_loop=180896) match mfb and Python. */
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "interpbench.h"

typedef enum { PUSH, LOAD, STORE, ADD, SUB, MUL, MODULO, LT, JZ, JMP, HALT } Op;

typedef struct {
  Op op;
  long long arg;
} Instr;

typedef struct {
  Instr *code;
  int len, cap;
} Prog;

static long long run_vm(const Prog *p) {
  long long stack[64] = {0};
  long long vars[8] = {0}; /* both programs use <= 6 variable slots */
  int sp = 0, pc = 0;
  for (;;) {
    const Instr *ins = &p->code[pc++];
    switch (ins->op) {
      case PUSH: stack[sp++] = ins->arg; break;
      case LOAD: stack[sp++] = vars[ins->arg]; break;
      case STORE: vars[ins->arg] = stack[--sp]; break;
      case ADD: sp--; stack[sp - 1] = stack[sp - 1] + stack[sp]; break;
      case SUB: sp--; stack[sp - 1] = stack[sp - 1] - stack[sp]; break;
      case MUL: sp--; stack[sp - 1] = stack[sp - 1] * stack[sp]; break;
      case MODULO: sp--; stack[sp - 1] = stack[sp - 1] % stack[sp]; break;
      case LT: sp--; stack[sp - 1] = stack[sp - 1] < stack[sp] ? 1 : 0; break;
      case JZ: if (stack[--sp] == 0) pc = (int)ins->arg; break;
      case JMP: pc = (int)ins->arg; break;
      case HALT: return stack[sp - 1];
    }
  }
}

/* ----- program assembly (untimed) ----------------------------------------- */

static int emit(Prog *p, Op op, long long arg) {
  if (p->len == p->cap) {
    p->cap = p->cap ? p->cap * 2 : 64;
    p->code = realloc(p->code, (size_t)p->cap * sizeof *p->code);
  }
  p->code[p->len].op = op;
  p->code[p->len].arg = arg;
  return p->len++;
}

/* Variables: 0 acc, 1 r, 2 a, 3 b, 4 k, 5 t. */
static Prog fib_program(long long reps, long long n) {
  const long long P = 1000000007LL;
  Prog p = {0};
  emit(&p, PUSH, 0); emit(&p, STORE, 0);
  emit(&p, PUSH, 0); emit(&p, STORE, 1);
  int outer = p.len;
  emit(&p, LOAD, 1); emit(&p, PUSH, reps); emit(&p, LT, 0);
  int exit_outer = emit(&p, JZ, 0);
  emit(&p, PUSH, 0); emit(&p, STORE, 2);
  emit(&p, PUSH, 1); emit(&p, STORE, 3);
  emit(&p, PUSH, 0); emit(&p, STORE, 4);
  int inner = p.len;
  emit(&p, LOAD, 4); emit(&p, PUSH, n); emit(&p, LT, 0);
  int exit_inner = emit(&p, JZ, 0);
  emit(&p, LOAD, 2); emit(&p, LOAD, 3); emit(&p, ADD, 0);
  emit(&p, PUSH, P); emit(&p, MODULO, 0); emit(&p, STORE, 5);
  emit(&p, LOAD, 3); emit(&p, STORE, 2);
  emit(&p, LOAD, 5); emit(&p, STORE, 3);
  emit(&p, LOAD, 4); emit(&p, PUSH, 1); emit(&p, ADD, 0); emit(&p, STORE, 4);
  emit(&p, JMP, inner);
  p.code[exit_inner].arg = p.len;
  emit(&p, LOAD, 0); emit(&p, LOAD, 2); emit(&p, ADD, 0);
  emit(&p, LOAD, 1); emit(&p, ADD, 0);
  emit(&p, PUSH, P); emit(&p, MODULO, 0); emit(&p, STORE, 0);
  emit(&p, LOAD, 1); emit(&p, PUSH, 1); emit(&p, ADD, 0); emit(&p, STORE, 1);
  emit(&p, JMP, outer);
  p.code[exit_outer].arg = p.len;
  emit(&p, LOAD, 0); emit(&p, HALT, 0);
  return p;
}

/* Variables: 0 acc, 1 i, 2 j. */
static Prog loop_program(long long rows, long long cols) {
  Prog p = {0};
  emit(&p, PUSH, 0); emit(&p, STORE, 0);
  emit(&p, PUSH, 0); emit(&p, STORE, 1);
  int outer = p.len;
  emit(&p, LOAD, 1); emit(&p, PUSH, rows); emit(&p, LT, 0);
  int exit_outer = emit(&p, JZ, 0);
  emit(&p, PUSH, 0); emit(&p, STORE, 2);
  int inner = p.len;
  emit(&p, LOAD, 2); emit(&p, PUSH, cols); emit(&p, LT, 0);
  int exit_inner = emit(&p, JZ, 0);
  emit(&p, LOAD, 0); emit(&p, LOAD, 1); emit(&p, LOAD, 2); emit(&p, MUL, 0);
  emit(&p, ADD, 0); emit(&p, PUSH, 1); emit(&p, ADD, 0);
  emit(&p, PUSH, 1000003); emit(&p, MODULO, 0); emit(&p, STORE, 0);
  emit(&p, LOAD, 2); emit(&p, PUSH, 1); emit(&p, ADD, 0); emit(&p, STORE, 2);
  emit(&p, JMP, inner);
  p.code[exit_inner].arg = p.len;
  emit(&p, LOAD, 1); emit(&p, PUSH, 1); emit(&p, ADD, 0); emit(&p, STORE, 1);
  emit(&p, JMP, outer);
  p.code[exit_outer].arg = p.len;
  emit(&p, LOAD, 0); emit(&p, HALT, 0);
  return p;
}

/* ----- rows ----------------------------------------------------------------- */

static void run_row(const char *name, const char *key, Prog prog) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    bench_opaque(prog.code);
    long long t0 = now_ns();
    checksum = run_vm(&prog);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "%s = %lld\n", key, checksum);
  record("interp", name, t, RUN);
  free(t);
  free(prog.code);
}

void run_interp_group(void) {
  run_row("vm", "test_interp_vm", fib_program(1500, 100));
  run_row("vm_loop", "test_interp_vm_loop", loop_program(400, 400));
}
