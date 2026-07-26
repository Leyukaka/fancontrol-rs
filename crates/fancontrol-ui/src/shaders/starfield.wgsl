// Starfield / particle tunnel: domain-repeated small spheres faking a stream
// of particles rushing past the camera. Heat-reactive: cell size shrinks
// (denser swarm) and travel speed rises as heat01 increases; the shared
// raymarch's heat-tint blend also pushes star color from cool white/blue
// toward warm orange/red, a direct "color temperature" pun.

fn map(p_in: vec3<f32>) -> f32 {
    let heat = uniforms.heat01;
    let cell = mix(6.0, 3.0, heat);
    let speed = 10.0 + heat * 30.0;

    var p = p_in;
    p.z = p.z + uniforms.time * speed;

    let q = vec3<f32>(
        p.x - cell * round(p.x / cell),
        p.y - cell * round(p.y / cell),
        p.z - cell * round(p.z / cell),
    );
    return (length(q) - 0.3) * 0.5;
}
