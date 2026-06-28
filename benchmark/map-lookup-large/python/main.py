N = 20000
m = {}
for i in range(N):
    m[i] = i
s = 0
for i in range(N):
    s += m[i]
print(f"count: {N} sum: {s}")
