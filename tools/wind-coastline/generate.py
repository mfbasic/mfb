#!/usr/bin/env python3
"""Build examples/wind/resources/wind/coastline.bin from Natural Earth's 1:10m land.

Every outline is simplified with Visvalingam-Whyatt on the projected (Equal Earth)
plane, and every point keeps the importance it had when it was removed, so the app
can take any level of detail out of one file by keeping the points at or above a
threshold. See README.md for the file format and why it is shaped this way.

Usage: generate.py <ne_10m_land.geojson> <out coastline.bin>
"""

import heapq
import json
import math
import sys

# --- Equal Earth, exactly as examples/wind/src/earth.mfb `project` computes it ---
A1, A2, A3, A4 = 1.340264, -0.081106, 0.000893, 0.003796
M = math.sqrt(3.0) / 2.0


def project(lon, lat):
    lam = math.radians(max(-180.0, min(180.0, lon)))
    phi = math.radians(max(-90.0, min(90.0, lat)))
    theta = math.asin(max(-1.0, min(1.0, M * math.sin(phi))))
    t2 = theta * theta
    t6 = t2 * t2 * t2
    denominator = A1 + 3.0 * A2 * t2 + t6 * (7.0 * A3 + 9.0 * A4 * t2)
    return (lam * math.cos(theta) / (M * denominator),
            theta * (A1 + A2 * t2 + t6 * (A3 + A4 * t2)))


# --- Importance levels ---
# A point's importance is the area (plane units squared) of the triangle it made with
# its neighbours when Visvalingam removed it. The file keeps it as a byte: four levels
# per doubling of area, level 1 at 2^-35. Below 2^-35 a point is under 0.05 square
# pixels even at the app's deepest zoom on a Retina-wide window (~30,000 px per plane
# unit), so it is dropped from the file entirely.
MIN_LOG2_AREA = -35
LEVELS_PER_DOUBLING = 4
COORD_SCALE = 10000  # 1e-4 degree, ~11 m; ~0.05 px at the deepest zoom


def level_of(area):
    if area <= 0.0:
        return 0
    level = int(math.floor((math.log2(area) - MIN_LOG2_AREA) * LEVELS_PER_DOUBLING)) + 1
    return max(0, min(255, level))


def triangle_area(a, b, c):
    return abs((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) * 0.5


def importance(plane):
    """Visvalingam-Whyatt effective area per point of a closed ring (no repeated end).

    Areas are made monotone (a point removed after another never scores lower), so
    keeping every point with area >= T reproduces the simplification exactly at T. The
    last three points score the area of the triangle they leave, raised to the running
    maximum, so a whole small island drops out at once as the threshold passes it.
    """
    n = len(plane)
    prev = [(i - 1) % n for i in range(n)]
    nxt = [(i + 1) % n for i in range(n)]
    alive = [True] * n
    area = [0.0] * n
    stamp = [0] * n
    heap = []
    for i in range(n):
        heapq.heappush(heap, (triangle_area(plane[prev[i]], plane[i], plane[nxt[i]]), i, 0))
    remaining = n
    running = 0.0
    while remaining > 3:
        a, i, s = heapq.heappop(heap)
        if not alive[i] or s != stamp[i]:
            continue
        running = max(running, a)
        area[i] = running
        alive[i] = False
        remaining -= 1
        p, q = prev[i], nxt[i]
        nxt[p], prev[q] = q, p
        for j in (p, q):
            stamp[j] += 1
            heapq.heappush(heap, (triangle_area(plane[prev[j]], plane[j], plane[nxt[j]]), j, stamp[j]))
    last = [i for i in range(n) if alive[i]]
    final = max(running, triangle_area(*(plane[i] for i in last)))
    for i in last:
        area[i] = final
    return area


def varint(out, value):
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return


def zigzag(value):
    return (value << 1) if value >= 0 else ((-value << 1) - 1)


def outer_rings(geojson):
    holes = 0
    for feature in geojson["features"]:
        geometry = feature["geometry"]
        polygons = geometry["coordinates"] if geometry["type"] == "MultiPolygon" else [geometry["coordinates"]]
        for polygon in polygons:
            holes += len(polygon) - 1
            yield polygon[0], holes


SEAM = 180 * COORD_SCALE
POLE = 90 * COORD_SCALE

# Ring kinds, the per-ring flag byte in the file.
CLOSED = 0          # an ordinary closed outline; its longitudes may run past 180
SOUTH_POLAR = 1     # an open coast path spanning 360 degrees, closed through the pole


def seam_runs(coords):
    """Contiguous (cyclic) runs of point indices lying on longitude +-180."""
    n = len(coords)
    on = [k for k in range(n) if abs(coords[k][0]) == SEAM]
    runs = []
    for k in on:
        if runs and k == runs[-1][-1] + 1:
            runs[-1].append(k)
        else:
            runs.append([k])
    if len(runs) > 1 and runs[0][0] == 0 and runs[-1][-1] == n - 1:
        runs[0] = runs.pop() + runs[0]
    return runs


def cyclic(coords, start, stop):
    """coords[start], coords[start+1], ... up to and including coords[stop], cyclically."""
    n = len(coords)
    out = []
    k = start % n
    while True:
        out.append(coords[k])
        if k == stop % n:
            return out
        k = (k + 1) % n


def stitch(a, run_a, b, run_b):
    """Join ring `a` (cut along +180) and ring `b` (cut along -180) into one outline.

    Natural Earth splits a landmass that crosses the antimeridian into two rings that
    each walk the cut. The joined ring drops both walks: it follows `a` round to where
    it meets the cut, crosses into `b` (moved east by 360 degrees so the longitudes run
    on continuously), follows `b` round, and crosses back.
    """
    a0, a1 = run_a[0], run_a[-1]
    b0, b1 = run_b[0], run_b[-1]
    # The two walks run in opposite directions: a's first cut point meets b's last.
    tol = COORD_SCALE // 10
    assert abs(a[a0][1] - b[b1][1]) <= tol and abs(a[a1][1] - b[b0][1]) <= tol, "cuts do not meet"
    shifted = [(x + 2 * SEAM, y) for x, y in b]
    joined = cyclic(a, a1 + 1, a0 - 1) + [a[a0]] + cyclic(shifted, b1, b0) + [a[a1]]
    return joined


def polar_path(coords, runs):
    """Antarctica: the coast between its two cut walks, west to east."""
    (r0, r1) = runs
    # One walk runs up +180 to the pole, the other down -180 from it; the coast is the
    # stretch between the end of the -180 walk and the start of the +180 walk.
    west = r0 if coords[r0[0]][0] == -SEAM else r1
    east = r1 if west is r0 else r0
    path = cyclic(coords, west[-1], east[0])
    assert path[0][0] == -SEAM and path[-1][0] == SEAM, "polar ring is not cut at both edges"
    return path


def dedupe(coords):
    return [c for k, c in enumerate(coords) if k == 0 or c != coords[k - 1]]


def ranked(coords, kind):
    """Importance levels for a ring, projected about its own middle longitude."""
    lons = [x for x, _ in coords]
    middle = (min(lons) + max(lons)) / 2.0
    closed = coords if kind == CLOSED else coords + [(coords[-1][0], -POLE), (coords[0][0], -POLE)]
    plane = [project((x - middle) / COORD_SCALE, y / COORD_SCALE) for x, y in closed]
    levels = [level_of(a) for a in importance(plane)]
    if kind == SOUTH_POLAR:
        levels = levels[:len(coords)]
        # The path's ends are where it meets the cut; the app closes it from there.
        levels[0] = levels[-1] = 255
    return levels


def main():
    source, target = sys.argv[1], sys.argv[2]
    with open(source) as f:
        geojson = json.load(f)

    rings = []
    total_points = holes = 0
    for ring, holes in outer_rings(geojson):
        total_points += len(ring)
        coords = [(round(lon * COORD_SCALE), round(lat * COORD_SCALE)) for lon, lat in ring]
        if len(coords) > 1 and coords[0] == coords[-1]:
            coords.pop()
        coords = dedupe(coords)
        if len(coords) >= 3:
            rings.append(coords)

    # Pair the rings cut along the antimeridian, and find the polar one.
    out_rings = []
    east_cut, west_cut = [], []
    stitched = polar = 0
    for coords in rings:
        runs = seam_runs(coords)
        if not runs:
            out_rings.append((CLOSED, coords))
        elif len(runs) == 2 and any(y == -POLE for _, y in coords):
            out_rings.append((SOUTH_POLAR, dedupe(polar_path(coords, runs))))
            polar += 1
        elif len(runs) == 1 and coords[runs[0][0]][0] == SEAM:
            east_cut.append((coords, runs[0]))
        elif len(runs) == 1:
            west_cut.append((coords, runs[0]))
        else:
            raise SystemExit(f"a ring meets the antimeridian in {len(runs)} places; not handled")
    for a, run_a in east_cut:
        lats_a = sorted(a[k][1] for k in run_a)
        match = None
        for i, (b, run_b) in enumerate(west_cut):
            lats_b = sorted(b[k][1] for k in run_b)
            if abs(lats_a[0] - lats_b[0]) <= COORD_SCALE // 10 and abs(lats_a[-1] - lats_b[-1]) <= COORD_SCALE // 10:
                match = i
                break
        if match is None:
            raise SystemExit("a ring cut along +180 has no partner along -180")
        b, run_b = west_cut.pop(match)
        out_rings.append((CLOSED, dedupe(stitch(a, run_a, b, run_b))))
        stitched += 1
    if west_cut:
        raise SystemExit("a ring cut along -180 has no partner along +180")

    out = bytearray(b"MFBC")
    out.append(2)  # format version
    varint(out, len(out_rings))
    kept_points = 0
    for kind, coords in out_rings:
        levels = ranked(coords, kind)
        kept = [(c, lv) for c, lv in zip(coords, levels) if lv > 0]
        out.append(kind)
        varint(out, len(kept))
        kept_points += len(kept)
        last_x = last_y = 0
        for (x, y), lv in kept:
            varint(out, zigzag(x - last_x))
            varint(out, zigzag(y - last_y))
            out.append(lv)
            last_x, last_y = x, y
    with open(target, "wb") as f:
        f.write(out)
    print(f"rings={len(out_rings)} stitched={stitched} polar={polar} points={kept_points} "
          f"of {total_points} holes_skipped={holes} bytes={len(out)}")


if __name__ == "__main__":
    main()
