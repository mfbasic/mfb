/* The C peer for recordbench.mfb (P1 coverage): record-shaped workloads the way
 * each language writes them.
 *
 *   record wide / nested — a 10-field mixed long/double/string struct passed and
 *     returned BY VALUE through a function in a hot loop; `nested` holds it
 *     inside another struct inside an array (read element, update, write back).
 *     Strings are `const char *` (C's idiom — the pointer is copied, not bytes).
 *   float nbody_rec / nbody_vec — the SAME simulation as `float nbody` in main.c
 *     (same bodies, 100000 steps, same pair and float op order) over a struct
 *     array, and over a struct of two 3-double vectors with dot/distance helpers.
 *
 * Checksums match mfb/Python: every record-row double op is an add or a
 * multiply by a power of two (exact, contraction-proof), and the n-body rows
 * fold llround(energy * 1e9) — the nine decimals `float nbody` prints. */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "recordbench.h"

/* ===================================================================== */
/* GROUP: record (wide / nested)                                         */
/* ===================================================================== */

typedef struct {
  long id, count, total, flags;
  double pos, vel, energy;
  const char *name, *tag;
  double weight;
} Wide;

typedef struct {
  long id;
  const char *label;
  Wide inner;
  long touches;
} Holder;

static Wide step_wide(Wide w, long i, const char *tag) {
  Wide n = w;
  n.count = w.count + 1;
  n.total = w.total + i % 7;
  n.flags = (w.flags * 5 + i) % 1024;
  n.pos = w.pos + w.vel * 0.25;
  n.vel = w.vel * 0.5 + (double)(i % 4);
  n.energy = w.energy + w.vel;
  n.tag = tag;
  return n;
}

static long wide_sum(Wide w) {
  return w.id + w.count + w.total + w.flags + (long)floor(w.pos * 4.0) + (long)floor(w.vel * 1024.0) +
         (long)floor(w.energy) + (long)strlen(w.name) + (long)strlen(w.tag) + (long)floor(w.weight * 8.0);
}

static Wide new_wide(long id, char *namebuf, size_t cap) {
  snprintf(namebuf, cap, "wide-%ld", id);
  Wide w = {id, 0, id, id % 1024, (double)id * 0.5, 1.0, 0.0, namebuf, "", (double)id * 0.125};
  return w;
}

static void test_record_wide(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char namebuf[32];
  for (int r = 0; r < RUN; r++) {
    Wide w = new_wide(7, namebuf, sizeof namebuf);
    long long t0 = now_ns();
    for (long i = 0; i < 1000000; i++) {
      if (i % 2 == 0) w = step_wide(w, i, "even-step");
      else w = step_wide(w, i, "odd");
    }
    t[r] = now_ns() - t0;
    checksum = wide_sum(w);
  }
  fprintf(stderr, "test_record_wide = %ld\n", checksum);
  record("record", "wide", t, RUN);
  free(t);
}

#define NHOLD 64
static void test_record_nested(void) {
  long long *t = alloc_times();
  long checksum = 0;
  static char names[NHOLD][32], labels[NHOLD][32];
  Holder holders[NHOLD];
  for (int r = 0; r < RUN; r++) {
    for (int k = 0; k < NHOLD; k++) {
      snprintf(labels[k], sizeof labels[k], "holder-%d", k);
      Holder h = {k, labels[k], new_wide(k, names[k], sizeof names[k]), 0};
      holders[k] = h;
    }
    long long t0 = now_ns();
    for (long i = 0; i < 300000; i++) {
      long k = i % NHOLD;
      Holder h = holders[k];
      h.inner = step_wide(h.inner, i, i % 2 == 0 ? "even-step" : "odd");
      h.touches = h.touches + 1;
      holders[k] = h;
    }
    bench_opaque(holders);
    t[r] = now_ns() - t0;
    long sum = 0;
    for (int k = 0; k < NHOLD; k++)
      sum += holders[k].id + (long)strlen(holders[k].label) + holders[k].touches + wide_sum(holders[k].inner);
    checksum = sum;
  }
  fprintf(stderr, "test_record_nested = %ld\n", checksum);
  record("record", "nested", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: float (nbody_rec / nbody_vec)                                  */
/* ===================================================================== */

#define NB 5
typedef struct { double x, y, z, vx, vy, vz, mass; } Body;

static void nbody_bodies(Body b[NB]) {
  const double PI = 3.141592653589793;
  const double SOLAR_MASS = 4.0 * PI * PI;
  const double DPY = 365.24;
  Body init[NB] = {
    {0.0, 0.0, 0.0, 0.0, 0.0, 0.0, SOLAR_MASS},
    {4.84143144246472090, -1.16032004402742839, -1.03622044471123109e-01,
     1.66007664274403694e-03 * DPY, 7.69901118419740425e-03 * DPY, -6.90460016972063023e-05 * DPY,
     9.54791938424326609e-04 * SOLAR_MASS},
    {8.34336671824457987, 4.12479856412430479, -4.03523417114321381e-01,
     -2.76742510726862411e-03 * DPY, 4.99852801234917238e-03 * DPY, 2.30417297573763929e-05 * DPY,
     2.85885980666130812e-04 * SOLAR_MASS},
    {1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
     2.96460137564761618e-03 * DPY, 2.37847173959480950e-03 * DPY, -2.96589568540237556e-05 * DPY,
     4.36624404335156298e-05 * SOLAR_MASS},
    {1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
     2.68067772490389322e-03 * DPY, 1.62824170038242295e-03 * DPY, -9.51592254519715870e-05 * DPY,
     5.15138902046611451e-05 * SOLAR_MASS},
  };
  double px = 0.0, py = 0.0, pz = 0.0;
  for (int i = 0; i < NB; i++) { px += init[i].vx * init[i].mass; py += init[i].vy * init[i].mass; pz += init[i].vz * init[i].mass; }
  init[0].vx = -px / SOLAR_MASS; init[0].vy = -py / SOLAR_MASS; init[0].vz = -pz / SOLAR_MASS;
  memcpy(b, init, sizeof init);
}

static void test_nbody_rec(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  const double dt = 0.01;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    Body b[NB];
    nbody_bodies(b);
    for (int s = 0; s < 100000; s++) {
      for (int i = 0; i < NB; i++) {
        for (int j = i + 1; j < NB; j++) {
          double dx = b[i].x - b[j].x, dy = b[i].y - b[j].y, dz = b[i].z - b[j].z;
          double d2 = dx * dx + dy * dy + dz * dz;
          double mag = dt / (d2 * sqrt(d2));
          b[i].vx -= dx * b[j].mass * mag; b[i].vy -= dy * b[j].mass * mag; b[i].vz -= dz * b[j].mass * mag;
          b[j].vx += dx * b[i].mass * mag; b[j].vy += dy * b[i].mass * mag; b[j].vz += dz * b[i].mass * mag;
        }
      }
      for (int i = 0; i < NB; i++) { b[i].x += dt * b[i].vx; b[i].y += dt * b[i].vy; b[i].z += dt * b[i].vz; }
    }
    double e = 0.0;
    for (int i = 0; i < NB; i++) {
      e += 0.5 * b[i].mass * (b[i].vx * b[i].vx + b[i].vy * b[i].vy + b[i].vz * b[i].vz);
      for (int j = i + 1; j < NB; j++) {
        double dx = b[i].x - b[j].x, dy = b[i].y - b[j].y, dz = b[i].z - b[j].z;
        e -= b[i].mass * b[j].mass / sqrt(dx * dx + dy * dy + dz * dz);
      }
    }
    checksum = llround(e * 1000000000.0);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_float_nbody_rec = %lld\n", checksum);
  record("float", "nbody_rec", t, RUN);
  free(t);
}

typedef struct { double x, y, z; } Vec3;
typedef struct { Vec3 pos, vel; double mass; } VBody;

static inline double vec3_dot(Vec3 a, Vec3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
static inline double vec3_distance(Vec3 a, Vec3 b) {
  Vec3 d = {a.x - b.x, a.y - b.y, a.z - b.z};
  return sqrt(vec3_dot(d, d));
}

static void test_nbody_vec(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  const double dt = 0.01;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    Body init[NB];
    nbody_bodies(init);
    VBody b[NB];
    for (int i = 0; i < NB; i++) {
      VBody v = {{init[i].x, init[i].y, init[i].z}, {init[i].vx, init[i].vy, init[i].vz}, init[i].mass};
      b[i] = v;
    }
    for (int s = 0; s < 100000; s++) {
      for (int i = 0; i < NB; i++) {
        for (int j = i + 1; j < NB; j++) {
          Vec3 d = {b[i].pos.x - b[j].pos.x, b[i].pos.y - b[j].pos.y, b[i].pos.z - b[j].pos.z};
          double d2 = vec3_dot(d, d);
          double mag = dt / (d2 * sqrt(d2));
          Vec3 vi = {b[i].vel.x - d.x * b[j].mass * mag, b[i].vel.y - d.y * b[j].mass * mag, b[i].vel.z - d.z * b[j].mass * mag};
          b[i].vel = vi;
          Vec3 vj = {b[j].vel.x + d.x * b[i].mass * mag, b[j].vel.y + d.y * b[i].mass * mag, b[j].vel.z + d.z * b[i].mass * mag};
          b[j].vel = vj;
        }
      }
      for (int i = 0; i < NB; i++) {
        Vec3 p = {b[i].pos.x + dt * b[i].vel.x, b[i].pos.y + dt * b[i].vel.y, b[i].pos.z + dt * b[i].vel.z};
        b[i].pos = p;
      }
    }
    double e = 0.0;
    for (int i = 0; i < NB; i++) {
      e += 0.5 * b[i].mass * vec3_dot(b[i].vel, b[i].vel);
      for (int j = i + 1; j < NB; j++) e -= b[i].mass * b[j].mass / vec3_distance(b[i].pos, b[j].pos);
    }
    checksum = llround(e * 1000000000.0);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_float_nbody_vec = %lld\n", checksum);
  record("float", "nbody_vec", t, RUN);
  free(t);
}

void run_recordbench_float_rows(void) {
  test_nbody_rec();
  test_nbody_vec();
}

void run_recordbench_record_rows(void) {
  test_record_wide();
  test_record_nested();
}
