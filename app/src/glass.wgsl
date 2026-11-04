// Liquid glass: a rounded pane that bends, blurs and lights up what lies
// behind it. "Behind" is the window's background, drawn a second time into
// a texture of its own before the frame (see background.rs).

struct Pane {
    // The color laid over the glass, and how much of it (a).
    tint: vec4<f32>,
    // bend (px the edge pulls in), frost (blur radius, px), dispersion, rim.
    look: vec4<f32>,
    // corner radius as a share of half the short side, opacity,
    // 1.0 when the target re-encodes sRGB, how much the backdrop is lifted.
    shape: vec4<f32>,
    // The window in pixels, and 1.0 if there is a backdrop at all.
    screen: vec4<f32>,
    // A flat color standing in for the backdrop (a = how much).
    flat: vec4<f32>,
};

@group(0) @binding(0) var backdrop: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(1) @binding(0) var<uniform> pane: Pane;

struct Out {
    @builtin(position) pos: vec4<f32>,
    // 0..1 across the pane's rect (the viewport is set to it).
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.local = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

fn behind(px: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(backdrop, smp, px / pane.screen.xy, 0.0).rgb;
    return mix(c, pane.flat.rgb, pane.flat.a);
}

// The backdrop at `px`, blurred over `radius` pixels (a golden-angle spiral).
fn frosted(px: vec2<f32>, radius: f32) -> vec3<f32> {
    if (radius < 0.5) {
        return behind(px);
    }
    var sum = behind(px);
    var weight = 1.0;
    for (var k = 1; k < 16; k++) {
        let f = f32(k);
        let r = sqrt(f / 15.0) * radius;
        let a = f * 2.39996;
        let w = 1.0 - 0.5 * f / 15.0;
        sum += behind(px + vec2<f32>(cos(a), sin(a)) * r) * w;
        weight += w;
    }
    return sum / weight;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    // The pane's size in pixels, from how fast `local` changes.
    let size = vec2<f32>(1.0 / abs(dpdx(in.local.x)), 1.0 / abs(dpdy(in.local.y)));
    let half = size * 0.5;
    let pos = (in.local - 0.5) * size;
    let r = clamp(pane.shape.x, 0.0, 1.0) * min(half.x, half.y);

    // Signed distance to the rounded rect (negative inside) and its normal.
    let q = abs(pos) - half + r;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    let s = vec2<f32>(select(-1.0, 1.0, pos.x >= 0.0), select(-1.0, 1.0, pos.y >= 0.0));
    var n: vec2<f32>;
    if (q.x > 0.0 && q.y > 0.0) {
        n = s * normalize(q);
    } else if (q.x > q.y) {
        n = vec2<f32>(s.x, 0.0);
    } else {
        n = vec2<f32>(0.0, s.y);
    }
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    // Refraction: like a thick lens, the rim pulls the picture in from
    // further inside; the middle stays true.
    let inside = max(-d, 0.0);
    let band = max(min(max(r, 14.0), min(half.x, half.y)), 1.0);
    let t = clamp(inside / band, 0.0, 1.0);
    let lens = pow(1.0 - t, 2.2);
    let pull = n * pane.look.x * lens;
    // Dispersion: red bends a little less than blue.
    let spread = pane.look.z * 0.35 * lens;
    let frost = pane.look.y;
    let at = in.pos.xy;
    // Without the background behind it (a whole-screen style fills the
    // window), the glass is see-through instead: what is really drawn under
    // it shows, darkened by the tint, and only the light is added on top.
    let clear = pane.screen.z < 0.5 || pane.flat.a > 0.5;
    var col: vec3<f32>;
    if (!clear) {
        col = vec3<f32>(
            frosted(at - pull * (1.0 - spread), frost).r,
            frosted(at - pull, frost).g,
            frosted(at - pull * (1.0 + spread), frost).b,
        );
    } else {
        col = vec3<f32>(0.0);
    }

    // Work in sRGB-encoded values, like the rest of the UI.
    let linear_target = pane.shape.z > 0.5;
    if (linear_target) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }

    // Glass makes colors behind it a little richer and brighter.
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(vec3<f32>(luma), col, 1.25);
    col = col * (1.0 + pane.shape.w) + pane.shape.w * 0.08;
    // How much of the pane is glass body (the rest lets the picture through).
    var body = 1.0;
    if (clear) {
        body = clamp(pane.tint.a, 0.2, 0.8);
        col = pane.tint.rgb;
    } else {
        col = mix(col, pane.tint.rgb, pane.tint.a);
    }
    col *= body;

    // Light on the rim: brightest where it faces the light (top left) and
    // the opposite corner, as on curved glass.
    let light = normalize(vec2<f32>(-0.55, -1.0));
    let facing = abs(dot(n, light));
    let spec = pow(facing, 2.5);
    let rim = pane.look.w;
    let edge = smoothstep(2.2, 0.0, inside);
    let glow = exp(-inside / max(band * 0.35, 1.0));
    var light_add = rim * edge * (0.35 + 1.4 * spec) + rim * 0.45 * glow * spec;
    // A soft sheen falling from the top, as light lies on a curved pane.
    let top = clamp(in.local.y, 0.0, 1.0);
    light_add += rim * 0.22 * pow(1.0 - top, 3.0) * smoothstep(0.0, 3.0, inside);
    // A faint darker line just inside the rim gives it thickness.
    col *= 1.0 - 0.12 * rim * smoothstep(1.5, 3.0, inside) * smoothstep(6.0, 3.0, inside);
    col += vec3<f32>(light_add);

    // `col` is premultiplied by `body` already; the light is added on top.
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    if (linear_target) {
        col = pow(col, vec3<f32>(2.2));
    }
    let k = coverage * pane.shape.y;
    return vec4<f32>(col * k, body * k);
}
