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
// ring arc: 线宽沿弧向位置 t 连续插值（weightProp(t) ∈ [u_ring_min_w, u_ring_max_w] × u_ring_w_mul）
uniform float u_ring_min_w;
uniform float u_ring_max_w;
uniform float u_ring_w_mul;
uniform float u_aa;      // antialias half-pixel (px)
uniform float u_a0;      // ring arc start angle (radians)
uniform float u_a1;      // ring arc end angle (radians)
uniform vec2  u_p0;      // trail seg start / triangle vertex 0 (geo px)
uniform vec2  u_p1;      // trail seg end / triangle vertex 1 (geo px)
uniform vec2  u_p2;      // triangle vertex 2 (geo px)
uniform float u_trail_a0; // trail endpoint alpha (start) [legacy, kept compat]
uniform float u_trail_a1; // trail endpoint alpha (end)   [legacy, kept compat]

// ─── Trail polyline (root-cause fix for segment-junction gaps).
// Single quad envelopes the ENTIRE trail path. Shader computes min signed
// distance to the whole polyline as ONE continuous distance field — no
// per-segment butt-cap boundaries, so halo & core stay continuous across
// every junction. Mirrors BASpark's single Canvas2D beginPath→lineTo*→stroke()
// call (lineJoin=miter keeps junctions seamless). prior per-segment capsules
// had `if (t<0||t>1) return 1e10` caps that zeroed halo at each junction →
// the "thin line breaks" symptom.
const int MAX_TRAIL_PTS = 128;
uniform float u_trail_count;                 // # of polyline points (as float, _1f)
uniform vec2  u_trail_pts[MAX_TRAIL_PTS];    // polyline pts (geo px, quad-local)

// 1px-aware coverage from a signed distancefield (matches niri convention).
// Coverage from a signed distancefield, opposite niri_scale (per-pixel) scaling.
float aa_cov(float d, float aa) {
    // Bug1 fix: 仓库标准 AA 模式 (rounding_alpha.frag:25) — niri_scale 应
    // 乘在被比较的 d 上 (logical->physical) 而非 smoothstep 阈值。原实现
    // smoothstep(-aa*ns, +aa*ns, d) 让软边膨胀为 2*aa*ns^2 物理 px，
    // HiDPI 下 ring 爆裂段比 BASpark 扩 3 倍宽。
    // 正确: d 乘 niri_scale 转物理，半物理像素 aa (=0.5) -> 1 物理 px 软边。
    float t = clamp(d * niri_scale + aa, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// SDF of a filled circle (BASpark ctx.arc + fill).
float sd_circle(vec2 p, vec2 c, float r) {
    return length(p - c) - r;
}

// BASpark `index.html:420-422` weightProp(t) = min(2 - |4(t-0.5)|, 1)：
// 弧向位置 t∈[0,1] 两侧到中央的描边宽度峰。
float weight_prop(float t) {
    return min(2.0 - abs(4.0 * (t - 0.5)), 1.0);
}

// SDF of a circular arc stroke with CONTINUOUS width along the arc.
// Line width at angular position t = weightProp(t) interpolated in
// [min_w, max_w] × w_mul. This replaces the previous discrete per-sub-segment
// width which made rings look like faceted polygons (jagged edges).
// Handles arbitrary a0 < a1 (may cross the ±π boundary) by normalizing to
// [0, 2π) and measuring the forward arc length from a0. Round caps at the
// true endpoints. (B7 fix: BASpark ctx.arc accepts any angle range directly.)
float sd_arc(vec2 p, vec2 c, float r, float a0, float a1, float min_w, float max_w, float w_mul) {
    vec2 d = p - c;
    float ang = atan(d.y, d.x);
    const float TWO_PI = 6.2831853;
    float na  = mod(ang + TWO_PI, TWO_PI);   // atan() ∈ [-π,π] → [0,2π)
    float na0 = mod(a0  + TWO_PI, TWO_PI);
    float arc_len = a1 - a0;                 // > 0 by construction
    float pos = mod(na - na0 + TWO_PI, TWO_PI); // forward distance from na0
    float t = clamp(pos / max(arc_len, 1e-5), 0.0, 1.0);
    float w = mix(min_w, max_w, weight_prop(t)) * w_mul;
    float sd;
    if (pos <= arc_len) {
        sd = abs(length(d) - r) - w * 0.5;
    } else {
        vec2 cap0 = c + vec2(cos(a0), sin(a0)) * r;
        vec2 cap1 = c + vec2(cos(a1), sin(a1)) * r;
        // 端帽使用端点的线宽（t=0/1 → min_w）
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
// Bug2 fix: 平端 (butt caps) capsule — 段内画圆角矩形(lineWidth=w)，端点外不画
// 任何半圆端帽。配合加法混合避免相邻段端帽在交界点 4x 叠加产生高亮点。
// BASpark Canvas2D: 全 trail 单条 beginPath -> lineTo -> stroke() 一次描边,
// default lineCap='butt' + 段间端点共享无重叠 -> 无 junction 高亮。
float sd_capsule_butt(vec2 p, vec2 a, vec2 b, float w) {
    vec2 pa = p - a, ba = b - a;
    float bb = max(dot(ba, ba), 1e-5);
    float t = dot(pa, ba) / bb;        // 不 clamp：端点外侧由下方判定排除
    if (t < 0.0 || t > 1.0) return 1e10;
    return length(pa - ba * t) - w * 0.5;
}

// 蓝色光晕（BASpark "lighter" 泛光外观）：形状外 soft falloff。与形状 core 做
// 预乘 over 合成（result = core + glow*(1-core)），普通混合下亮背景不饱和、
// 边缘依旧平滑圆润。
float glow_cov(float sd, float strength, float radius) {
    return strength * (1.0 - smoothstep(0.0, radius, max(sd, 0.0)));
}

void main() {
    vec3 coords_geo3 = input_to_geo * vec3(niri_v_coords, 1.0);
    vec2 p = coords_geo3.xy;

    float a = 0.0;
    vec3 rgb = u_color;

    if (u_mode < 0.5) {
        // filledCircle + 蓝色光晕
        float sd = sd_circle(p, u_center, u_radius);
        float core = aa_cov(sd, u_aa);
        float glow = glow_cov(sd, 0.6, 5.5);
        a = core + glow * (1.0 - core);
    } else if (u_mode < 1.5) {
        // ring arc stroke — 线宽沿弧连续变化（weightProp），边缘平滑圆润 + 蓝色光晕
        float sd = sd_arc(p, u_center, u_radius, u_a0, u_a1, u_ring_min_w, u_ring_max_w, u_ring_w_mul);
        float core = aa_cov(sd, u_aa);
        float glow = glow_cov(sd, 0.6, 5.5);
        a = core + glow * (1.0 - core);
    } else if (u_mode < 2.5) {
        // spark triangle (BASpark: rgba(255,255,255,alpha)) + 光晕
        float sd = sd_triangle(p, u_p0, u_p1, u_p2);
        float core = aa_cov(sd, u_aa);
        float glow = glow_cov(sd, 0.4, 3.0);
        a = core + glow * (1.0 - core);
        rgb = vec3(1.0);
    } else {
        // trail: SINGLE polyline distance field (root-cause fix).
        // BASpark _updateTrail strokes one Canvas2D path (beginPath → lineTo*
        // → stroke()); lineJoin=miter keeps junctions seamless. We replicate by
        // computing min distance to all segments + tracking global parameter
        // t∈[0,1] along the whole polyline (BASpark's segGrad stitches into a
        // pure linear 0→1 gradient, so alpha_at_t = best_t exactly — no per-point
        // alpha array needed and no GL ES 2.0 dynamic-index violation).
        // core: 5px hard (radius 2.5, 1-px AA). halo: 3px gaussian falloff
        // 0.6 @ d=0 → 0 @ d=5.5 (shadowBlur=3, shadowColor=rgba(color,0.6)).
        // trail does NOT multiply opacity (BASpark skips this.alpha() for trail).
        int n = int(u_trail_count + 0.5);
        if (n >= 2) {
            float best_d = 1e10;
            float best_t = 0.0;
            for (int i = 0; i < MAX_TRAIL_PTS - 1; i++) {
                if (i >= n - 1) break;
                vec2 sa = u_trail_pts[i];
                vec2 sb = u_trail_pts[i + 1];
                vec2 pa = p - sa;
                vec2 ba = sb - sa;
                float bb = max(dot(ba, ba), 1e-5);
                float t = clamp(dot(pa, ba) / bb, 0.0, 1.0);
                float d = length(pa - ba * t);
                if (d < best_d) {
                    best_d = d;
                    float seg0 = float(i)      / float(n - 1);
                    float seg1 = float(i + 1)  / float(n - 1);
                    best_t = mix(seg0, seg1, t);
                }
            }
            float core_cov = aa_cov(best_d - 2.5, u_aa);
            float halo_cov = 0.6 * (1.0 - smoothstep(0.0, 5.5, best_d));
            // 预乘 over 合成（与 niri 其余元素一致，非加法混合）：halo 在下、core 在上，
            // 同色（u_color）下 result = core + halo*(1-core)。保证 alpha∈[0,1]，
            // 亮背景上不饱和 → AA 软边正确显示（加法混合会饱和成硬边/锯齿）。
            float core_a = core_cov * best_t;
            float halo_a = halo_cov * best_t;
            a = core_a + halo_a * (1.0 - core_a);
        }
    }

    float alpha = a * niri_alpha;
    gl_FragColor = vec4(rgb * alpha, alpha);

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.3) + gl_FragColor * 0.7;
#endif
}
