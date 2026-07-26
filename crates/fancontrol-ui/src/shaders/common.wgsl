// Shared uniform layout, vertex shader, math helpers, and raymarch loop for
// the graph-style shader gallery. Each style file (pyramid.wgsl, plasma.wgsl,
// lava_blob.wgsl, starfield.wgsl) only defines its own scene function:
//
//   fn map(p: vec3<f32>) -> f32
//
// It can read `uniforms.time` / `uniforms.heat01` / `uniforms.cpu01` /
// `uniforms.gpu01` / `uniforms.params` freely as globals (WGSL functions can
// reference module-scope declarations regardless of textual order). This
// file is concatenated in front of the style file's source at build time
// (see shaders/mod.rs) into a single shader module per style.
//
// Notes on WGSL vs. the original GLSL this was ported from:
// - WGSL forbids assigning to a multi-component swizzle (`p.xz = ...`); every
//   style file must assign components individually (`p.x = ...; p.z = ...;`).
// - `@builtin(position)` has y=0 at the top of the framebuffer; flipped in
//   `fs_main` below to match the bottom-up convention raymarching shaders
//   usually assume.
// - Output alpha is forced to 1.0 in `fs_main`; none of these shaders use
//   alpha for real blending against the rest of the UI.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    cpu01: f32,
    gpu01: f32,
    heat01: f32,
    _pad0: vec2<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

var<private> big_triangle: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
);

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(big_triangle[v_idx], 0.0, 1.0);
}

fn rotate(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(p.x * c + p.y * s, -p.x * s + p.y * c);
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

fn raymarch(ro: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    var t: f32 = 0.0;
    var col = vec3<f32>(0.0, 0.0, 0.0);
    var d: f32 = 0.0;
    let hot = vec3<f32>(1.0, 0.35, 0.08);
    for (var i: i32 = 0; i < 64; i = i + 1) {
        let p = ro + rd * t;
        d = map(p) * 0.5;
        if (d < 0.02) {
            break;
        }
        if (d > 100.0) {
            break;
        }
        let base = mix(uniforms.color_a.rgb, uniforms.color_b.rgb, length(p) * 0.1);
        // Hotter hardware nudges the palette toward an angry orange/red on
        // top of whichever colors the user picked, so temperature always
        // visibly affects the look, not just decorative motion.
        let shaded = mix(base, hot, uniforms.heat01 * 0.5);
        col = col + shaded / (400.0 * max(d, 0.0001));
        t = t + d;
    }
    return col;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let frag_coord = vec2<f32>(pos.x, uniforms.resolution.y - pos.y);
    let uv = (frag_coord - uniforms.resolution * 0.5) / uniforms.resolution.x;

    var ro = vec3<f32>(0.0, 0.0, -50.0);
    let ro_xz = rotate(vec2<f32>(ro.x, ro.z), uniforms.time);
    ro.x = ro_xz.x;
    ro.z = ro_xz.y;

    let cf = normalize(-ro);
    let cs = normalize(cross(cf, vec3<f32>(0.0, 1.0, 0.0)));
    let cu = normalize(cross(cf, cs));

    let uuv = ro + cf * 3.0 + uv.x * cs + uv.y * cu;
    let rd = normalize(uuv - ro);

    let col = raymarch(ro, rd);
    return vec4<f32>(col, 1.0);
}
