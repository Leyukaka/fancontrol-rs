// Lava / liquid blob: a few spheres blended with smooth-min, bobbing and
// drifting like a lava lamp. Heat-reactive: bobbing/merge speed rises with
// heat, and the smooth-min blend factor loosens so blobs look "meltier".

fn map(p_in: vec3<f32>) -> f32 {
    let heat = uniforms.heat01;
    let speed = 0.3 + heat * 1.2;
    let k = mix(1.2, 3.0, heat);
    let t = uniforms.time * speed;

    let p1 = p_in - vec3<f32>(sin(t) * 6.0, cos(t * 1.3) * 4.0, 0.0);
    let p2 = p_in - vec3<f32>(cos(t * 0.8) * 5.0, sin(t * 1.7) * 5.0, sin(t) * 3.0);
    let p3 = p_in - vec3<f32>(sin(t * 1.1) * 4.0, sin(t * 0.6) * 6.0, cos(t * 0.9) * 4.0);

    let d1 = length(p1) - 6.0;
    let d2 = length(p2) - 5.0;
    let d3 = length(p3) - 4.0;

    var d = smin(d1, d2, k);
    d = smin(d, d3, k);
    return d * 0.5;
}
