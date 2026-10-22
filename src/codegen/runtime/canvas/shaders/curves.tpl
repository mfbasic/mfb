// plan-164-C: the curve shapes (`helper_curves.rs`). A `PolyCurve` (kind 11) and a
// `CurvePolygon` (kind 12) are drawn from y-monotone rational-quadratic PIECES, which
// ride the edge region two slots a piece: piece `e`'s chord `x0, y0, x2, y2` in slot
// `arc.z + e` -- where the band index reads it as an edge, a piece's two ends being its
// whole y-range -- and its control point and weight in slot `arc.z + misc.w + e`, as `x1, y1`, the fp32 BITS
// of `w1` and a zero. Each function below is the oracle's of the same name
// (`__canvas_pieceNearest`, `__canvas_pieceCrossX`, `__canvas_pieceCut`, ...), in
// fp32, on coordinates taken RELATIVE TO THE PIXEL first. The two shaders' text is
// generated from one template (`tools/canvas-curve-shader`), so they are line for line.
struct MfbPiece { vec2 p0; vec2 p1; vec2 p2; float w; };
struct MfbPieceQ { vec2 a; vec2 b; vec2 c; float wb; };
@S MfbPiece curvePiece(@D int e, vec2 pt) {
  int s = item.arc.z + e;
  int c = s + item.misc.w;
  MfbPiece k;
  k.p0 = curvePoint(@C s, 0) - pt;
  k.p2 = curvePoint(@C s, 2) - pt;
  k.p1 = curvePoint(@C c, 0) - pt;
  k.w = curveWeight(@C c);
  return k;
}
// With Q(t) = a t^2 + b t + c the piece's numerator less the pixel times its
// denominator W(t) = 1 + wb t (1 - t), the squared distance is Q.Q / W^2 and its slope
// has the sign of g = (Q.Q')W - (Q.Q)W'. `order` 0 is g, 1 its derivative, 2 its second.
@S float pcF(MfbPieceQ k, int order, float t) {
  vec2 q = (k.a * t + k.b) * t + k.c;
  vec2 d = 2.0 * k.a * t + k.b;
  float w = 1.0 + k.wb * t * (1.0 - t);
  float dw = k.wb * (1.0 - 2.0 * t);
  float qd = dot(q, d);
  if (order == 0) { return qd * w - dot(q, q) * dw; }
  if (order == 1) { return (dot(d, d) + 2.0 * dot(q, k.a)) * w - qd * dw + 2.0 * k.wb * dot(q, q); }
  return 6.0 * dot(d, k.a) * w + 6.0 * k.wb * qd;
}
// The root between `lo` and `hi`, where the function changes sign: Newton's step when it
// lands in the bracket -- its ends included, so a step that has arrived stays -- and the
// midpoint when not. Twelve steps, `__CANVAS_PIECE_STEPS`.
@S float pcRoot(MfbPieceQ k, int order, float lo, float hi) {
  bool rising = pcF(k, order, lo) < 0.0;
  float t = (lo + hi) * 0.5;
  for (int i = 0; i < 12; ++i) {
    float f = pcF(k, order, t);
    if ((f < 0.0) == rising) { lo = t; } else { hi = t; }
    float slope = pcF(k, order + 1, t);
    float mid = (lo + hi) * 0.5;
    if (abs(f) < abs(slope)) {
      float moved = t - f / slope;
      if (moved >= lo && moved <= hi) { mid = moved; }
    }
    t = mid;
  }
  return t;
}
// `best` is (squared distance, parameter); `t` as a candidate for it.
@S vec2 pcTry(MfbPieceQ k, float t, vec2 best) {
  vec2 q = (k.a * t + k.b) * t + k.c;
  float w = 1.0 + k.wb * t * (1.0 - t);
  float d2 = dot(q, q) / (w * w);
  return d2 < best.x ? vec2(d2, t) : best;
}
// (distance, parameter of the nearest point) of a piece from the pixel. The roots are
// isolated, not searched for: g'' is a quadratic, fitted through its values at 0, 1/2
// and 1 and solved; between its roots g' is monotone and holds at most one root; between
// those g is monotone, and a stretch that goes from negative to positive holds exactly
// one minimum.
@S vec2 pieceNearest(MfbPiece p) {
  MfbPieceQ k;
  vec2 q1 = p.w * p.p1;
  k.a = p.p0 - 2.0 * q1 + p.p2;
  k.b = 2.0 * (q1 - p.p0);
  k.c = p.p0;
  k.wb = 2.0 * (p.w - 1.0);
  vec2 best = vec2(dot(p.p0, p.p0), 0.0);
  vec3 h = vec3(0.0);
  for (int i = 0; i < 3; ++i) { h[i] = pcF(k, 2, 0.5 * float(i)); }
  float alpha = 2.0 * (h.x + h.z) - 4.0 * h.y;
  float beta = h.z - h.x - alpha;
  float r1 = 2.0;
  float r2 = 2.0;
  float disc = beta * beta - 4.0 * alpha * h.x;
  if (disc >= 0.0) {
    float s = sqrt(disc);
    float q = beta < 0.0 ? -(beta - s) * 0.5 : -(beta + s) * 0.5;
    if (abs(q) < abs(alpha) && (q > 0.0) == (alpha > 0.0) && q != 0.0) { r1 = q / alpha; }
    if (abs(h.x) < abs(q) && (h.x > 0.0) == (q > 0.0) && h.x != 0.0) {
      float other = h.x / q;
      if (other < r1) { r2 = r1; r1 = other; } else { r2 = other; }
    }
  }
  vec4 turn = vec4(2.0, 2.0, 2.0, 1.0);
  vec3 from = vec3(0.0, r1, r2);
  vec3 to = vec3(min(r1, 1.0), min(r2, 1.0), 1.0);
  for (int i = 0; i < 3; ++i) {
    if (from[i] < 1.0) {
      float flo = pcF(k, 1, from[i]);
      float fhi = pcF(k, 1, to[i]);
      if ((flo < 0.0 && fhi > 0.0) || (flo > 0.0 && fhi < 0.0)) { turn[i] = pcRoot(k, 1, from[i], to[i]); }
    }
  }
  float lo = 0.0;
  for (int i = 0; i < 4; ++i) {
    float hi = turn[i];
    if (hi > 1.5) { continue; }
    if (pcF(k, 0, lo) < 0.0 && pcF(k, 0, hi) > 0.0) { best = pcTry(k, pcRoot(k, 0, lo, hi), best); }
    best = pcTry(k, hi, best);
    lo = hi;
  }
  return vec2(sqrt(best.x), best.y);
}
// A bound below the piece's distance: the pixel's distance to the box round its three
// control points.
@S float pieceBound(MfbPiece p) {
  vec2 lo = min(min(p.p0, p.p1), p.p2);
  vec2 hi = max(max(p.p0, p.p1), p.p2);
  return length(max(max(lo, -hi), vec2(0.0)));
}
// What a piece adds to the pixel's winding: 0 unless its ends are on opposite sides of
// the pixel's row and it passes LEFT of the pixel there; then +1 running down the screen
// and -1 running up. The crossing is the root of (a - 2b + c) t^2 + 2 (b - a) t + a taken
// as a / (a - b + s), s = sqrt(b^2 - a c) signed as (a - c): the form that divides by
// the larger candidate.
@S int pieceCrossing(MfbPiece p) {
  float a = p.p0.y;
  float c = p.p2.y;
  if ((a > 0.0) == (c > 0.0)) { return 0; }
  float t = 0.0;
  if (a != 0.0) {
    float b = p.w * p.p1.y;
    float s = sqrt(b * b - a * c);
    t = a / (a - b + (a < c ? -s : s));
  }
  t = clamp(t, 0.0, 1.0);
  float u = 1.0 - t;
  float k1 = 2.0 * p.w * u * t;
  float xr = (u * u * p.p0.x + k1 * p.p1.x + t * t * p.p2.x) / (u * u + k1 + t * t);
  if (!(0.0 < xr)) { return 0; }
  return c > a ? 1 : -1;
}
// One end of a piece as (along, side, has a direction): how far the pixel is along the
// curve's direction there, from the end, and how far from the tangent line.
@S vec3 pieceEnd(MfbPiece p, bool atEnd) {
  vec2 e = atEnd ? p.p2 : p.p0;
  vec2 t = atEnd ? p.p2 - p.p1 : p.p1 - p.p0;
  if (t.x == 0.0 && t.y == 0.0) { t = p.p2 - p.p0; }
  float len2 = dot(t, t);
  if (!(len2 > 0.0)) { return vec3(0.0); }
  float len_ = sqrt(len2);
  return vec3(-dot(e, t) / len_, abs(e.y * t.x - e.x * t.y) / len_, 1.0);
}
// `__canvas_pieceCut`: one end of a piece's band, cut as `cap` says (0 Butt, 2 Square),
// given its round band there and whether the nearest point of the piece IS that end.
@S float pieceCut(MfbPiece p, float halfW, bool atEnd, int cap, float band, bool nearest) {
  vec3 f = pieceEnd(p, atEnd);
  if (f.z == 0.0) {
    if (!nearest) { return band; }
    if (cap == 2) { return max(abs(p.p0.x), abs(p.p0.y)) - halfW; }
    return 1.0e6;
  }
  float plane = atEnd ? f.x : -f.x;
  if (cap == 2) { plane -= halfW; }
  if (nearest) { return cap == 2 ? max(f.y - halfW, plane) : max(band, plane); }
  float zone = halfW * 1.5 + 2.0;
  if (plane > -1.0 && f.x * f.x + f.y * f.y <= zone * zone) { return max(band, plane); }
  return band;
}
// A piece's outline position at `t`, from its arc-length row (`row` in the dash words):
// the position at each sixteenth of the piece, read off the straight line between two.
@S float pieceArc(@D int row, float t) {
  float f = t * 16.0;
  int k = int(clamp(floor(f), 0.0, 15.0));
  float c0 = dashWord(@C row + k);
  float c1 = dashWord(@C row + k + 1);
  return c0 + (c1 - c0) * (f - float(k));
}
// The point of a piece at outline position `s`, less the pixel.
@S vec2 pieceAtArc(@D MfbPiece p, int row, float s) {
  int lo = 0;
  int hi = 16;
  for (int i = 0; i < 4; ++i) {
    int mid = (lo + hi) / 2;
    if (dashWord(@C row + mid) <= s) { lo = mid; } else { hi = mid; }
  }
  float c0 = dashWord(@C row + lo);
  float c1 = dashWord(@C row + lo + 1);
  float f = c1 > c0 ? clamp((s - c0) / (c1 - c0), 0.0, 1.0) : 0.0;
  float t = (float(lo) + f) / 16.0;
  float u = 1.0 - t;
  float k1 = 2.0 * p.w * u * t;
  return (u * u * p.p0 + k1 * p.p1 + t * t * p.p2) / (u * u + k1 + t * t);
}
// Every distance a curve shape is drawn from, in one walk over the band's pieces so
// that `pieceNearest` has one call site. `mode` 0 is a CurvePolygon's signed distance
// (`__canvas_curveDistance`); 1 a PolyCurve's stroke (`__canvas_curveScan`); 2 a
// CurvePolygon's stroke with join records (`__canvas_curveJoined`); 3 either's dashed
// stroke (`__canvas_dashedCurves`).
@S float curveWalk(@D ivec2 band, vec2 pt, int mode, float halfW, int cap) {
  int total = item.misc.w;
  bool joins = item.strokeStyle.y > 0;
  int inner = joins ? 0 : 1;
  float reachW = mode == 0 ? 0.0 : halfW * 1.4142135;
  float best = 1.0e6;
  int winding = 0;
  int data = 0;
  float phase = 0.0;
  float ext = cap == 2 ? halfW : 0.0;
  if (mode == 3) {
    data = 3 + int(dashWord(@C 1));
    phase = dashWord(@C 0);
    // A chain that is a single point is the dot its cap draws (`__canvas_dashDot`).
    if (total == 1 && !(dashWord(@C data + 16) > dashWord(@C data))) {
      MfbPiece dot0 = curvePiece(@C 0, pt);
      if (cap == 0 || item.misc.x != 11 || !dashOnAt(@C phase)) { return 1.0e6; }
      if (cap == 2) { return max(abs(dot0.p0.x), abs(dot0.p0.y)) - halfW; }
      return length(dot0.p0) - halfW;
    }
  }
  for (int j = 0; j < band.y; ++j) {
    int e = band.x < 0 ? j : @E[band.x + j];
    MfbPiece p = curvePiece(@C e, pt);
    if (mode == 0) { winding += pieceCrossing(p); }
    int row = data + e * 18;
    float s0 = 0.0;
    float s1 = 0.0;
    float ring = 0.0;
    if (mode == 3) {
      s0 = dashWord(@C row);
      s1 = dashWord(@C row + 16);
      ring = dashWord(@C row + 17);
      if (!(s1 > s0)) { continue; }
    }
    if (pieceBound(p) - reachW < best) {
      vec2 n = pieceNearest(p);
      float d = n.x - (mode == 0 ? 0.0 : halfW);
      if (mode == 3) {
        vec3 f = vec3(0.0);
        if (n.y <= 0.0) { f = pieceEnd(p, false); } else if (n.y >= 1.0) { f = pieceEnd(p, true); }
        float u = 0.0;
        if (f.z != 0.0) {
          u = (n.y <= 0.0 ? s0 : s1) + f.x;
          d = f.y - halfW;
        } else {
          u = pieceArc(@C row, n.y);
        }
        vec2 ends;
        bvec2 has;
        d = max(d, dashAlong(@C u, s0, s1, abs(ring), ring > 0.0, ext, ends, has));
        if (cap == 1) {
          if (has.x) { d = min(d, length(pieceAtArc(@C p, row, ends.x)) - halfW); }
          if (has.y) { d = min(d, length(pieceAtArc(@C p, row, ends.y)) - halfW); }
        }
      } else if (mode != 0) {
        int startCap = mode == 2 ? 0 : (e == 0 ? cap : inner);
        int endCap = mode == 2 ? 0 : (e == total - 1 ? cap : inner);
        if (startCap != 1) { d = pieceCut(p, halfW, false, startCap, d, n.y <= 0.0); }
        if (endCap != 1) { d = pieceCut(p, halfW, true, endCap, d, n.y >= 1.0); }
      }
      best = min(best, d);
    }
    // The record at this piece's start: a disc where the outline is smooth, the paint's
    // join where two curves meet at an angle. Dashed, only where a dash runs through.
    if (mode != 0 && joins) {
      int jk = joinKind(@C e);
      bool drawn = jk != 0;
      if (drawn && mode == 3) {
        float before = (ring > 0.0 && s0 == 0.0) ? abs(ring) : s0;
        drawn = dashOnBefore(@C before + phase) && dashOnAt(@C s0 + phase);
      }
      if (drawn) {
        best = min(best, joinPieceAt(@C e, jk, pt - joinPoint(@C e, 0)));
      }
    }
  }
  if (mode != 0) { return best; }
  bool inside = item.arc.y != 0 ? winding != 0 : (winding & 1) != 0;
  return inside ? -best : best;
}
