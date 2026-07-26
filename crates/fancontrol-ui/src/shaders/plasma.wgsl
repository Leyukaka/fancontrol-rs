// Plasma / energy field: layered sine turbulence around a soft volumetric
// shell, raymarched with the shared accumulation loop in common.wgsl.
// Heat-reactive: hotter hardware raises turbulence frequency/speed and adds
// a flicker once heat01 crosses ~0.85 ("agitated" look).

fn map(p: vec3<f32>) -> f32 {
    let heat = uniforms.heat01;
    let freq = 1.0 + heat * 2.0;
    let speed = uniforms.time * (0.5 + heat * 1.5);
    let n = sin(p.x * freq + speed)
        + sin(p.y * freq + speed * 1.3)
        + sin(p.z * freq + speed * 0.7);
    let flicker = select(0.0, sin(uniforms.time * 20.0) * 0.15, heat > 0.85);
    return (length(p) - 8.0 - n * (1.2 + heat * 1.8) + flicker) * 0.5;
}
