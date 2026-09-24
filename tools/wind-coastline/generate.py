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


def main():
    source, target = sys.argv[1], sys.argv[2]
    with open(source) as f:
        geojson = json.load(f)

    rings_out = []
    kept_points = total_points = holes = 0
    for ring, holes in outer_rings(geojson):
        coords = [(round(lon * COORD_SCALE), round(lat * COORD_SCALE)) for lon, lat in ring]
        if len(coords) > 1 and coords[0] == coords[-1]:
            coords.pop()
        # Consecutive duplicates after quantisation make zero-area triangles for nothing.
        deduped = [c for k, c in enumerate(coords) if k == 0 or c != coords[k - 1]]
        total_points += len(ring)
        if len(deduped) < 3:
            continue
        plane = [project(x / COORD_SCALE, y / COORD_SCALE) for x, y in deduped]
        levels = [level_of(a) for a in importance(plane)]
        kept = [(c, lv) for c, lv in zip(deduped, levels) if lv > 0]
        if len(kept) < 3:
            continue
        kept_points += len(kept)
        rings_out.append(kept)

    out = bytearray(b"MFBC")
    out.append(1)  # format version
    varint(out, len(rings_out))
    for kept in rings_out:
        varint(out, len(kept))
        last_x = last_y = 0
        for (x, y), lv in kept:
            varint(out, zigzag(x - last_x))
            varint(out, zigzag(y - last_y))
            out.append(lv)
            last_x, last_y = x, y
    with open(target, "wb") as f:
        f.write(out)
    print(f"rings={len(rings_out)} points={kept_points} of {total_points} "
          f"holes_skipped={holes} bytes={len(out)}")


if __name__ == "__main__":
    main()
