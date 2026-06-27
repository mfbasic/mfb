"""Stresses copy-on-update of records: build a 100-element list of (n, label)
tuples, then run 10 passes incrementing n of every record by reassigning a new
tuple back into the list. Prints the checksum (sum of all n fields)."""

recs = [(i, "p" + str(i)) for i in range(100)]

for _pass in range(10):
    for j in range(100):
        rec = recs[j]
        recs[j] = (rec[0] + 1, rec[1])

checksum = sum(rec[0] for rec in recs)
print("checksum: " + str(checksum))
