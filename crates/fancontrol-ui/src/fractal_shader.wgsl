// Raymarched "fractal pyramid" — ported from a GLSL/Shadertoy shader (see
// crates/fancontrol-ui/src/fractal.rs for the Rust-side pipeline setup).
//
// Differences from the GLSL source, and why:
// - `p * mat2(c,s,-s,c)` is written out as explicit scalar equations instead of
//   constructing a WGSL mat2x2, to avoid any row/column-major ambiguity between
//   the two shading languages.
// - fragCoord's Y axis is flipped: wgpu's @builtin(position) has y=0 at the top
//   of the framebuffer, Shadertoy's fragCoord has y=0 at the bottom.
// - Output alpha is forced to 1.0: the source shader's alpha formula is never
//   actually used by Shadertoy's opaque canvas, so blending against it verbatim
//   would render the fractal almost fully transparent inside egui.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    _pad0: f32,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
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

fn map(p_in: vec3<f32>, time: f32) -> f32 {
    var p = p_in;
    for (var i: i32 = 0; i < 8; i = i + 1) {
        let t = time * 0.2;
        // WGSL doesn't allow assigning to a multi-component swizzle (`p.xz = ...`),
        // unlike GLSL — assign each component individually instead.
        let r1 = rotate(vec2<f32>(p.x, p.z), t);
        p.x = r1.x;
        p.z = r1.y;
        let r2 = rotate(vec2<f32>(p.x, p.y), t * 1.89);
        p.x = r2.x;
        p.y = r2.y;
        p.x = abs(p.x);
        p.z = abs(p.z);
        p.x = p.x - 0.5;
        p.z = p.z - 0.5;
    }
    return dot(sign(p), p) / 5.0;
}

fn raymarch(ro: vec3<f32>, rd: vec3<f32>, time: f32, color_a: vec3<f32>, color_b: vec3<f32>) -> vec4<f32> {
    var t: f32 = 0.0;
    var col = vec3<f32>(0.0, 0.0, 0.0);
    var d: f32 = 0.0;
    for (var i: i32 = 0; i < 64; i = i + 1) {
        let p = ro + rd * t;
        d = map(p, time) * 0.5;
        if (d < 0.02) {
            break;
        }
        if (d > 100.0) {
            break;
        }
        col = col + mix(color_a, color_b, length(p) * 0.1) / (400.0 * d);
        t = t + d;
    }
    return vec4<f32>(col, 1.0 / (d * 100.0));
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

    let col = raymarch(ro, rd, uniforms.time, uniforms.color_a.rgb, uniforms.color_b.rgb);
    return vec4<f32>(col.rgb, 1.0);
}
