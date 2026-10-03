// Full-screen background: the blurred artwork, liquid-warped by layered noise,
// blended with a flowing gradient in the track's own colors.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    // 0..1, spikes on each (estimated) beat.
    pulse: f32,
    vivid: vec4<f32>,
    mid: vec4<f32>,
    dark: vec4<f32>,
    art_size: vec2<f32>,
    has_art: f32,
    // 1.0 when the render target applies sRGB encoding itself.
    srgb_target: f32,
    // The previous song's artwork, fading out while `fade` goes 0 -> 1.
    prev_size: vec2<f32>,
    prev_has_art: f32,
    fade: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var art_tex: texture_2d<f32>;
@group(0) @binding(2) var art_smp: sampler;
@group(0) @binding(3) var prev_tex: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the whole viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// Sin-free hash (Dave Hoskins): the classic fract(sin(..)) one loses
// precision on many GPUs and shows up as blocky artifacts.
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var value = 0.0;
    var amp = 0.5;
    for (var o = 0; o < 5; o++) {
        value += amp * noise(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        amp *= 0.5;
    }
    return value;
}

// Artwork of `size`, scaled to cover the screen, slowly breathing and warped.
fn art_uv(uv: vec2<f32>, aspect: f32, size: vec2<f32>, warp: vec2<f32>) -> vec2<f32> {
    let art_aspect = size.x / max(size.y, 1.0);
    var auv = uv - 0.5;
    if (aspect > art_aspect) {
        auv.y *= art_aspect / aspect;
    } else {
        auv.x *= aspect / art_aspect;
    }
    let zoom = 1.12 + 0.04 * sin(u.time * 0.08) + 0.01 * u.pulse;
    return auv / zoom + 0.5 + warp;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.resolution;
    let uv = frag.xy / res;
    let aspect = res.x / res.y;
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let t = u.time * 0.05;

    // Domain warping: noise displacing noise gives the liquid look.
    let q = vec2<f32>(fbm(p * 1.4 + vec2<f32>(0.0, t)), fbm(p * 1.4 + vec2<f32>(5.2, -t)));
    let r = vec2<f32>(
        fbm(p * 1.8 + 3.0 * q + vec2<f32>(1.7, 9.2) + t * 1.3),
        fbm(p * 1.8 + 3.0 * q + vec2<f32>(8.3, 2.8) - t * 1.1),
    );
    let warp = (r - 0.5) * (0.12 + 0.03 * u.pulse);

    // Crossfade from the previous artwork to the current one.
    let current = textureSample(art_tex, art_smp, art_uv(uv, aspect, u.art_size, warp)).rgb;
    let previous = textureSample(prev_tex, art_smp, art_uv(uv, aspect, u.prev_size, warp)).rgb;
    let w_current = u.has_art * u.fade;
    let w_previous = u.prev_has_art * (1.0 - u.fade);
    let art_amount = w_current + w_previous;
    let art = (current * w_current + previous * w_previous) / max(art_amount, 0.0001);

    // Gradient flow in the palette colors.
    let f = fbm(p * 1.1 + 2.5 * r + vec2<f32>(t, -t));
    var col = mix(u.dark.rgb, u.mid.rgb, smoothstep(0.25, 0.85, f));
    col = mix(col, u.vivid.rgb, smoothstep(0.55, 0.95, r.x) * 0.7);
    col = mix(col, art, art_amount * 0.6);

    // Beat glow from the center, vignette, and room for the foreground text.
    let d = length(p);
    col += u.vivid.rgb * u.pulse * 0.07 * exp(-d * d * 2.5);
    col *= mix(0.30, 1.0, smoothstep(1.2, 0.15, d));
    col *= 0.78;

    // A touch of film grain hides banding in the dark gradients.
    col += (hash(frag.xy + fract(u.time) * 91.0) - 0.5) * 0.025;
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));

    // Colors here are sRGB-encoded; undo that if the target re-encodes.
    if (u.srgb_target > 0.5) {
        col = pow(col, vec3<f32>(2.2));
    }
    return vec4<f32>(col, 1.0);
}
