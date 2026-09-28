"""Small stack bytecode VM (group `interp`).

Mirrors benchmark/mfb/src/interp.mfb. mfb fetches an Instr record (ENUM opcode
+ Integer operand) from a `List OF Instr`, dispatches with MATCH, and keeps its
operand stack in a `MUT List OF Integer` via collections::get/set. The Python
peer uses an IntEnum opcode in an (op, arg) tuple, an if/elif dispatch chain,
and a preallocated list stack. The same two programs are assembled once
(untimed), instruction-for-instruction identical to mfb:
  vm       iterative Fibonacci mod 1000000007, n = 100, repeated 1500 times
  vm_loop  a 400 x 400 nested counting loop, acc = (acc + i*j + 1) mod 1000003

Expected checksums: vm=993890033, vm_loop=180896.
"""
import sys
from enum import IntEnum

RUN = 1
now_ns = None
record = None


class Op(IntEnum):
    PUSH = 0
    LOAD = 1
    STORE = 2
    ADD = 3
    SUB = 4
    MUL = 5
    MODULO = 6
    LT = 7
    JZ = 8
    JMP = 9
    HALT = 10


def run_vm(prog, nvars):
    stack = [0] * 64
    vars_ = [0] * nvars
    sp = 0
    pc = 0
    while True:
        op, arg = prog[pc]
        pc += 1
        if op == Op.PUSH:
            stack[sp] = arg
            sp += 1
        elif op == Op.LOAD:
            stack[sp] = vars_[arg]
            sp += 1
        elif op == Op.STORE:
            sp -= 1
            vars_[arg] = stack[sp]
        elif op == Op.ADD:
            sp -= 1
            stack[sp - 1] = stack[sp - 1] + stack[sp]
        elif op == Op.SUB:
            sp -= 1
            stack[sp - 1] = stack[sp - 1] - stack[sp]
        elif op == Op.MUL:
            sp -= 1
            stack[sp - 1] = stack[sp - 1] * stack[sp]
        elif op == Op.MODULO:
            sp -= 1
            stack[sp - 1] = stack[sp - 1] % stack[sp]
        elif op == Op.LT:
            sp -= 1
            stack[sp - 1] = 1 if stack[sp - 1] < stack[sp] else 0
        elif op == Op.JZ:
            sp -= 1
            if stack[sp] == 0:
                pc = arg
        elif op == Op.JMP:
            pc = arg
        else:  # Op.HALT
            return stack[sp - 1]


# ---- program assembly (untimed) ---------------------------------------------

def emit(prog, op, arg=0):
    prog.append((op, arg))
    return len(prog) - 1


def patch(prog, at, target):
    prog[at] = (prog[at][0], target)


def fib_program(reps, n):
    # Variables: 0 acc, 1 r, 2 a, 3 b, 4 k, 5 t.
    P = 1000000007
    p = []
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 0)
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 1)
    outer = len(p)
    emit(p, Op.LOAD, 1); emit(p, Op.PUSH, reps); emit(p, Op.LT)
    exit_outer = emit(p, Op.JZ)
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 2)
    emit(p, Op.PUSH, 1); emit(p, Op.STORE, 3)
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 4)
    inner = len(p)
    emit(p, Op.LOAD, 4); emit(p, Op.PUSH, n); emit(p, Op.LT)
    exit_inner = emit(p, Op.JZ)
    emit(p, Op.LOAD, 2); emit(p, Op.LOAD, 3); emit(p, Op.ADD)
    emit(p, Op.PUSH, P); emit(p, Op.MODULO); emit(p, Op.STORE, 5)
    emit(p, Op.LOAD, 3); emit(p, Op.STORE, 2)
    emit(p, Op.LOAD, 5); emit(p, Op.STORE, 3)
    emit(p, Op.LOAD, 4); emit(p, Op.PUSH, 1); emit(p, Op.ADD); emit(p, Op.STORE, 4)
    emit(p, Op.JMP, inner)
    patch(p, exit_inner, len(p))
    emit(p, Op.LOAD, 0); emit(p, Op.LOAD, 2); emit(p, Op.ADD)
    emit(p, Op.LOAD, 1); emit(p, Op.ADD)
    emit(p, Op.PUSH, P); emit(p, Op.MODULO); emit(p, Op.STORE, 0)
    emit(p, Op.LOAD, 1); emit(p, Op.PUSH, 1); emit(p, Op.ADD); emit(p, Op.STORE, 1)
    emit(p, Op.JMP, outer)
    patch(p, exit_outer, len(p))
    emit(p, Op.LOAD, 0); emit(p, Op.HALT)
    return p


def loop_program(rows, cols):
    # Variables: 0 acc, 1 i, 2 j.
    p = []
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 0)
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 1)
    outer = len(p)
    emit(p, Op.LOAD, 1); emit(p, Op.PUSH, rows); emit(p, Op.LT)
    exit_outer = emit(p, Op.JZ)
    emit(p, Op.PUSH, 0); emit(p, Op.STORE, 2)
    inner = len(p)
    emit(p, Op.LOAD, 2); emit(p, Op.PUSH, cols); emit(p, Op.LT)
    exit_inner = emit(p, Op.JZ)
    emit(p, Op.LOAD, 0); emit(p, Op.LOAD, 1); emit(p, Op.LOAD, 2); emit(p, Op.MUL)
    emit(p, Op.ADD); emit(p, Op.PUSH, 1); emit(p, Op.ADD)
    emit(p, Op.PUSH, 1000003); emit(p, Op.MODULO); emit(p, Op.STORE, 0)
    emit(p, Op.LOAD, 2); emit(p, Op.PUSH, 1); emit(p, Op.ADD); emit(p, Op.STORE, 2)
    emit(p, Op.JMP, inner)
    patch(p, exit_inner, len(p))
    emit(p, Op.LOAD, 1); emit(p, Op.PUSH, 1); emit(p, Op.ADD); emit(p, Op.STORE, 1)
    emit(p, Op.JMP, outer)
    patch(p, exit_outer, len(p))
    emit(p, Op.LOAD, 0); emit(p, Op.HALT)
    return p


# ---- rows ---------------------------------------------------------------------

def run_row(name, prog, nvars):
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = run_vm(prog, nvars)
        times.append(now_ns() - t0)
    print("test_interp_%s = %d" % (name, checksum), file=sys.stderr)
    record("interp", name, times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    run_row("vm", fib_program(1500, 100), 6)
    run_row("vm_loop", loop_program(400, 400), 3)
