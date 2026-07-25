/* GROUP: encoding (encoding:: package coverage)
 *
 * Mirrors benchmark/mfb/src/encoding.mfb: base64/hex/percent encode-decode round
 * trips. base64 and hex use hand-rolled RFC 4648 codecs; percent uses the RFC
 * 3986 unreserved set (A-Za-z0-9-._~) with uppercase escapes, matching mfb's
 * encoding::percentEncode. Each checksum folds the decoded bytes (== the input)
 * plus the encoded length, so all three languages agree bit-for-bit. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "encodingbench.h"

static const char B64[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static const char HEX[] = "0123456789abcdef";

/* base64-encode `n` bytes into `out` (NUL-terminated); returns encoded length. */
static int b64_encode(const unsigned char *in, int n, char *out) {
  int o = 0;
  for (int i = 0; i < n; i += 3) {
    int rem = n - i;
    uint32_t b0 = in[i];
    uint32_t b1 = rem > 1 ? in[i + 1] : 0;
    uint32_t b2 = rem > 2 ? in[i + 2] : 0;
    uint32_t triple = (b0 << 16) | (b1 << 8) | b2;
    out[o++] = B64[(triple >> 18) & 0x3F];
    out[o++] = B64[(triple >> 12) & 0x3F];
    out[o++] = rem > 1 ? B64[(triple >> 6) & 0x3F] : '=';
    out[o++] = rem > 2 ? B64[triple & 0x3F] : '=';
  }
  out[o] = '\0';
  return o;
}

static int b64_val(char c) {
  if (c >= 'A' && c <= 'Z') return c - 'A';
  if (c >= 'a' && c <= 'z') return c - 'a' + 26;
  if (c >= '0' && c <= '9') return c - '0' + 52;
  if (c == '+') return 62;
  if (c == '/') return 63;
  return -1; /* '=' padding */
}

/* base64-decode `text` into `out`; returns decoded byte count. */
static int b64_decode(const char *text, unsigned char *out) {
  int len = (int)strlen(text), o = 0;
  for (int i = 0; i + 3 < len; i += 4) {
    int v0 = b64_val(text[i]), v1 = b64_val(text[i + 1]);
    int v2 = b64_val(text[i + 2]), v3 = b64_val(text[i + 3]);
    uint32_t triple = ((uint32_t)v0 << 18) | ((uint32_t)v1 << 12) |
                      ((v2 < 0 ? 0 : (uint32_t)v2) << 6) |
                      (v3 < 0 ? 0 : (uint32_t)v3);
    out[o++] = (triple >> 16) & 0xFF;
    if (text[i + 2] != '=') out[o++] = (triple >> 8) & 0xFF;
    if (text[i + 3] != '=') out[o++] = triple & 0xFF;
  }
  return o;
}

static void make_buf(unsigned char *buf, int n) {
  for (int i = 0; i < n; i++) buf[i] = (unsigned char)((i * 37 + 11) % 256);
}

/* base64 round-trip. Arena-gated in mfb (plan-44-J); the C mirror keeps the same
 * tiny counts so the table lines up. */
static void test_encoding_base64(void) {
  unsigned char buf[64], back[64];
  char enc[128];
  make_buf(buf, 64);
  int reps = 4;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) {
      int el = b64_encode(buf, 64, enc);
      int dl = b64_decode(enc, back);
      long sb = 0;
      for (int k = 0; k < dl; k++) sb += back[k];
      acc += sb + el;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "encoding_base64 = %ld\n", checksum);
  record("encoding", "base64", t, RUN);
  free(t);
}

static void test_encoding_hex(void) {
  enum { BUF = 512 };
  unsigned char buf[BUF], back[BUF];
  char enc[BUF * 2 + 1];
  make_buf(buf, BUF);
  int reps = 8;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) {
      for (int k = 0; k < BUF; k++) {
        enc[2 * k] = HEX[buf[k] >> 4];
        enc[2 * k + 1] = HEX[buf[k] & 0xF];
      }
      enc[2 * BUF] = '\0';
      int el = 2 * BUF;
      for (int k = 0; k < BUF; k++) {
        int hi = enc[2 * k], lo = enc[2 * k + 1];
        hi = hi <= '9' ? hi - '0' : (hi | 0x20) - 'a' + 10;
        lo = lo <= '9' ? lo - '0' : (lo | 0x20) - 'a' + 10;
        back[k] = (unsigned char)(hi * 16 + lo);
      }
      long sb = 0;
      for (int k = 0; k < BUF; k++) sb += back[k];
      acc += sb + el;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "encoding_hex = %ld\n", checksum);
  record("encoding", "hex", t, RUN);
  free(t);
}

static int is_unreserved(int c) {
  return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
         (c >= '0' && c <= '9') || c == '-' || c == '.' || c == '_' || c == '~';
}

/* percent-encode into `out`; returns encoded length. */
static int pct_encode(const char *s, char *out) {
  int o = 0;
  for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
    int c = *p;
    if (is_unreserved(c)) {
      out[o++] = (char)c;
    } else {
      static const char UHEX[] = "0123456789ABCDEF";
      out[o++] = '%';
      out[o++] = UHEX[(c >> 4) & 0xF];
      out[o++] = UHEX[c & 0xF];
    }
  }
  out[o] = '\0';
  return o;
}

/* percent-decode into `out` (raw bytes); returns decoded byte count. */
static int pct_decode(const char *s, unsigned char *out) {
  int o = 0;
  for (const char *p = s; *p;) {
    if (*p == '%' && p[1] && p[2]) {
      int hi = p[1], lo = p[2];
      hi = hi <= '9' ? hi - '0' : (hi | 0x20) - 'a' + 10;
      lo = lo <= '9' ? lo - '0' : (lo | 0x20) - 'a' + 10;
      out[o++] = (unsigned char)(hi * 16 + lo);
      p += 3;
    } else {
      out[o++] = (unsigned char)*p++;
    }
  }
  return o;
}

static void test_encoding_percent(void) {
  const char *src =
      "https://example.com/search?q=hello world&lang=en_US#section-1 (v2.0) 100% done";
  int reps = 16;
  char enc[512];
  unsigned char back[256];
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) {
      int el = pct_encode(src, enc);
      int dl = pct_decode(enc, back);
      long sb = 0;
      for (int k = 0; k < dl; k++) sb += back[k];
      acc += sb + el;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "encoding_percent = %ld\n", checksum);
  record("encoding", "percent", t, RUN);
  free(t);
}

void run_encoding_group(void) {
  test_encoding_base64();
  test_encoding_hex();
  test_encoding_percent();
}
