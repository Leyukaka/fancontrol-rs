// Fractal pyramid: folding-rotation SDF, ported from a GLSL/Shadertoy shader.
// See common.wgsl for the shared Uniforms/raymarch/vertex-shader plumbing.

fn map(p_in: vec3<f32>) -> f32 {
    var p = p_in;
    for (var i: i32 = 0; i < 8; i = i + 1) {
        let t = uniforms.time * 0.2;
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
