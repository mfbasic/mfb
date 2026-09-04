# DEC-03 spike — decoder collection-amplification DoS (json/regex/csv)

audit-3 DEC-03 (`planning/audit-3-decoders.md`), bug-510.

`json::parse` (and `regex` findAll / `csv::parse`) materialize the whole input as
a per-element collection with large per-element overhead.

```
python3 gen.py > /tmp/dec03.json
mfb build spikes/audit-3/DEC-03
/usr/bin/time -l ./spikes/audit-3/DEC-03/build/mfb_project.out /tmp/dec03.json
```

## Observed (defect present, lead-run)

```
bytes=1200000
maximum resident set size  1052622848      # ~1.05 GB from a 1.2 MB input (~875x)
```

## Expected

Peak memory within a small constant factor of the input size.
