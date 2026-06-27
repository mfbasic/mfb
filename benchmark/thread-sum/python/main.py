"""Sums 0..39,999,999 by splitting into four 10,000,000 chunks, each summed in
its own thread. Python's GIL serializes the worker loops, so there is no real
parallel speedup."""

import threading

CHUNK = 10000000


def sum_chunk(start, out, idx):
    total = 0
    i = start
    stop = start + CHUNK
    while i < stop:
        total += i
        i += 1
    out[idx] = total


def main():
    out = [0, 0, 0, 0]
    threads = []
    for k in range(4):
        t = threading.Thread(target=sum_chunk, args=(k * CHUNK, out, k))
        threads.append(t)
        t.start()
    for t in threads:
        t.join()
    print("total: " + str(sum(out)))
    return 0


if __name__ == "__main__":
    main()
