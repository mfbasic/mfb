# tools/wind-coastline

Builds `examples/wind/resources/wind/coastline.bin`, the coastline the wind example
draws, from Natural Earth's 1:10m land layer.

    curl -L -o ne_10m_land.geojson \
      https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_land.geojson
    python3 tools/wind-coastline/generate.py ne_10m_land.geojson \
      examples/wind/resources/wind/coastline.bin

Standard-library Python 3 only. The run that produced the checked-in file used a
download with sha256
`1ac90796408bc6ad6911d69448485d3c4dbf2190370080368a09976e1c9f7416` and reported
`rings=6836 points=434309 of 443787 holes_skipped=1 bytes=1907877`, in about 4 s.

Natural Earth is public domain (https://www.naturalearthdata.com/about/terms-of-use/).

## Why the file is shaped this way

The example draws the map at a level of detail that follows the view: at most
`coastBudget()` (2,500) coastline points on screen whether the window shows the
whole world or one island (`examples/wind/src/land.mfb`). To take any level of
detail out of one file, every point carries its **importance**: the area of the
triangle it formed with its neighbours when Visvalingam-Whyatt simplification
removed it.
- The simplification runs on the projected (Equal Earth) plane, so areas compare
  the way the map shows them.
- Areas are made monotone, so keeping every point at or above a threshold
  reproduces the simplification at that threshold exactly.
- A ring's last three points score the area of the triangle they leave, so a small
  island drops out whole once the threshold passes it.

The app counts the on-screen points per importance level and keeps levels from the
top down while the count stays within the budget.

`generate.py`'s `project` must match `project` in `examples/wind/src/earth.mfb`.
The app projects the stored longitude and latitude itself, so a mismatch cannot move
the map. It would only rank the points by slightly wrong areas. Regenerate the file
if the projection ever changes.

## Format (version 1)

All integers are LEB128 varints.
- **Header:** `"MFBC"`, a version byte (`1`), then the ring count.
- **Each ring:** its point count, then for each point:
  - the zigzag-varint change in longitude and in latitude from the previous point
    (the first from 0), in 1/10000 of a degree (about 11 m, 0.05 px at the deepest
    zoom);
  - one importance byte: `floor((log2(area) + 35) * 4) + 1`, clamped to 1..255.
- Rings are closed implicitly; the repeated end point is dropped.

Points whose importance falls below level 1 (2^-35 plane units squared, under 0.05
square pixels at the deepest zoom on a Retina-wide window) are left out of the file.

The layer's single interior ring (a hole) is skipped. Every other ring is an outer
outline filled as land.
