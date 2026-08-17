precision highp float;

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

uniform float niri_alpha;
uniform float niri_scale;
uniform vec2  niri_size;
varying vec2 niri_v_coords;

// Cursor-effect primitives. Coordinate convention mirrors `border.frag`:
// `coords_geo = input_to_geo * vec3(niri_v_coords, 1.0)` → geometry-pixel
// space (Canvas-style top-left origin). All `u_center` / `u_p*` are in this
// space. Without this matrix, niri_v_coords sits in a normalized [k, k+1]
// space and every SDF collapses → the whole quad lights up (= the visual
// "square" bug).
uniform mat3 input_to_geo;
uniform vec2 geo_size;

uniform float u_mode;    // 0=filledCircle, 1=ring arc stroke, 2=triangle spark, 3=trail capsule
uniform vec3  u_color;    // rgb 0..1 (ignored for sparks → white per BASpark)
uniform vec2  u_center;  // primitive center (geo px)
uniform float u_radius;  // px
uniform float u_inner_w; // stroke / band width (px)
uniform float u_aa;      // antialias half-pixel (px)
uniform float u_a0;      // ring arc start angle (radians)
uniform float u_a1;      // ring arc end angle (radians)
uniform vec2  u_p0;      // trail seg start / triangle vertex 0 (geo px)
uniform vec2  u_p1;      // trail seg end / triangle vertex 1 (geo px)
uniform vec2  u_p2;      // triangle vertex 2 (geo px)
uniform float u_trail_a0; // trail endpoint alpha (start)
uniform float u_trail_a1; // trail endpoint alpha (end)

// 1px-aware coverage from a signed distancefield (matches niri convention).
// Coverage from a signed distancefield, opposite niri_scale (per-pixel) scaling.
float aa_cov(float d, float aa) {
    // `niri_scale` (pixels-per-point) gives crisp anti-aliasing across DPI.
    return 1.0 - smoothstep(-aa * niri_scale, aa * niri_scale, d);
}

// SDF of a filled circle (BASpark ctx.arc + fill).
float sd_circle(vec2 p, vec2 c, float r) {
    return length(p - c) - r;
}

// SDF of a circular arc stroke band (BASpark ctx.arc + stroke lineWidth).
// Handles arbitrary a0 < a1 (may cross the ±π boundary) by normalizing to
// [0, 2π) and measuring the forward arc length from a0. Round caps at the
// true endpoints. (B7 fix: BASpark ctx.arc accepts any angle range directly.)
float sd_arc(vec2 p, vec2 c, float r, float w, float a0, float a1) {
    vec2 d = p - c;
    float ang = atan(d.y, d.x);
    const float TWO_PI = 6.2831853;
    float na  = mod(ang + TWO_PI, TWO_PI);   // atan() ∈ [-π,π] → [0,2π)
    float na0 = mod(a0  + TWO_PI, TWO_PI);
    float arc_len = a1 - a0;                 // > 0 by construction
    float pos = mod(na - na0 + TWO_PI, TWO_PI); // forward distance from na0
    float sd;
    if (pos <= arc_len) {
        sd = abs(length(d) - r) - w * 0.5;
    } else {
        vec2 cap0 = c + vec2(cos(a0), sin(a0)) * r;
        vec2 cap1 = c + vec2(cos(a1), sin(a1)) * r;
        sd = min(length(p - cap0), length(p - cap1)) - w * 0.5;
    }
    return sd;
}

// SDF of a triangle (BASpark sparks: moveTo/lineTo x3 + fill).
// Adapted from Inigo Quilez sdTriangle.
float sd_triangle(vec2 p, vec2 a, vec2 b, vec2 cy) {
    vec2 e0 = b - a, e1 = cy - b, e2 = a - cy;
    vec2 v0 = p - a, v1 = p - b, v2 = p - cy;
    vec2 pq0 = e0 * clamp(-dot(v0, e0) / max(dot(e0, e0), 1e-5), 0.0, 1.0) - v0;
    vec2 pq1 = e1 * clamp(-dot(v1, e1) / max(dot(e1, e1), 1e-5), 0.0, 1.0) - v1;
    vec2 pq2 = e2 * clamp(-dot(v2, e2) / max(dot(e2, e2), 1e-5), 0.0, 1.0) - v2;
    float s = sign(e0.x * e2.y - e0.y * e2.x);
    vec2 d = min(min(vec2(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
                    vec2(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x))),
                    vec2(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)));
    return -sqrt(max(d.x, 0.0)) * sign(d.y);
}

// SDF of a capsule / line segment stroke (BASpark trail lineWidth=5).
float sd_capsule(vec2 p, vec2 a, vec2 b, float w) {
    vec2 pa = p - a, ba = b - a;
    float t = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-5), 0.0, 1.0);
    return length(pa - ba * t) - w * 0.5;
}

void main() {
    vec3 coords_geo3 = input_to_geo * vec3(niri_v_coords, 1.0);
    vec2 p = coords_geo3.xy;

    float a = 0.0;
    vec3 rgb = u_color;

    if (u_mode < 0.5) {
        // filledCircle
        a = aa_cov(sd_circle(p, u_center, u_radius), u_aa);
    } else if (u_mode < 1.5) {
        // ring arc stroke
        a = aa_cov(sd_arc(p, u_center, u_radius, u_inner_w, u_a0, u_a1), u_aa);
    } else if (u_mode < 2.5) {
        // spark triangle (BASpark: rgba(255,255,255,alpha))
        a = aa_cov(sd_triangle(p, u_p0, u_p1, u_p2), u_aa);
        rgb = vec3(1.0);
    } else {
        // trail capsule with linear alpha gradient along the segment param
        vec2 pa = p - u_p0;
        vec2 ba = u_p1 - u_p0;
        float t = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-5), 0.0, 1.0);
        float sd = sd_capsule(p, u_p0, u_p1, u_inner_w);
        a = aa_cov(sd, u_aa) * ((1.0 - t) * u_trail_a0 + t * u_trail_a1);
    }

    float alpha = a * niri_alpha;
    gl_FragColor = vec4(rgb * alpha, alpha);

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.3) + gl_FragColor * 0.7;
#endif
}
