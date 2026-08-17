precision highp float;

uniform float niri_alpha;
uniform float niri_scale;

uniform vec2 niri_size;
varying vec2 niri_v_coords;

// Cursor-effect primitives (BASpark 1:1 translation, vector+SDF, no textures).
uniform float u_mode;       // 0=filled circle, 1=ring-arc, 2=triangle(solid), 3=trail-band
uniform vec3  u_color;      // rgb (linear 0..1, premultiplied by alpha in main)
uniform vec2  u_center;     // primitive center, in element-local coords
uniform float u_radius;     // circle/ring radius (px)
uniform float u_inner_w;    // ring band width / trail-band thickness
uniform float u_aa;         // anti-alias radius (px)
uniform float u_trail_a0;   // trail-band alpha at start endpoint
uniform float u_trail_a1;   // trail-band alpha at end endpoint

float aa_step(float d, float aa) {
    return 1.0 - smoothstep(-aa, aa, d);
}

void main() {
    // Element-local coords; flip Y so origin is top-left like Canvas2D.
    vec2 d = vec2(niri_v_coords.x, niri_size.y - niri_v_coords.y) - u_center;

    float a = 0.0;

    if (u_mode < 0.5) {
        // Filled circle (filledCircle): soft SDF edge.
        float sd = length(d) - u_radius;
        a = aa_step(sd, u_aa);
    } else if (u_mode < 1.5) {
        // Ring (outer + arc): band centered on u_radius, width u_inner_w.
        float sd = abs(length(d) - u_radius) - u_inner_w * 0.5;
        a = aa_step(sd, u_aa);
    } else if (u_mode < 2.5) {
        // Triangle (solid): vertex shader already clipped to the triangle quad; full alpha.
        a = 1.0;
    } else {
        // Trail band: alpha interpolated along the band (endpoints baked into the element
        // via two triangles per segment in collect_render_elements). Use average endpoint alpha.
        a = (u_trail_a0 + u_trail_a1) * 0.5;
    }

    // Premultiplied alpha output; additive-style glow is approximated by bright colors.
    vec3 rgb = u_color * a;
    float alpha = a * niri_alpha;
    gl_FragColor = vec4(rgb * alpha, alpha);
}
