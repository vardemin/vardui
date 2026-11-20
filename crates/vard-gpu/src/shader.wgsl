// Vard GPU renderer: one instanced quad per primitive, shapes are signed distance fields.
// Colors are straight RGBA in 0..1 (sRGB values, blended without linearization to match the
// headless rasterizer); the fragment outputs premultiplied alpha.

struct Globals {
    viewport: vec4<f32>, // width, height (px), 0, 0
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,       // quad to cover, px: x y w h
    @location(1) shape: vec4<f32>,      // SDF box, px: x y w h
    @location(2) radii: vec4<f32>,      // tl tr br bl
    @location(3) clip: vec4<f32>,       // x0 y0 x1 y1
    @location(4) clip_radii: vec4<f32>, // tl tr br bl (rounded clip on the clip rect)
    @location(5) color: vec4<f32>,
    @location(6) params: vec4<f32>,     // kind, a, b, c
    @location(7) uv: vec4<f32>,         // glyph: atlas x y (texels); image: dest origin + size
    @location(8) stops: vec4<u32>,      // gradient stops, packed 0xAABBGGRR
    @location(9) gline: vec4<f32>,      // gradient line x0 y0 x1 y1
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) @interpolate(flat) shape: vec4<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) clip: vec4<f32>,
    @location(4) @interpolate(flat) clip_radii: vec4<f32>,
    @location(5) @interpolate(flat) color: vec4<f32>,
    @location(6) @interpolate(flat) params: vec4<f32>,
    @location(7) @interpolate(flat) uv: vec4<f32>,
    @location(8) @interpolate(flat) stops: vec4<u32>,
    @location(9) @interpolate(flat) gline: vec4<f32>,
    @location(10) @interpolate(flat) rect: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = i.rect.xy + corner * i.rect.zw;
    var o: VOut;
    o.pos = vec4<f32>(p.x / g.viewport.x * 2.0 - 1.0, 1.0 - p.y / g.viewport.y * 2.0, 0.0, 1.0);
    o.p = p;
    o.shape = i.shape;
    o.radii = i.radii;
    o.clip = i.clip;
    o.clip_radii = i.clip_radii;
    o.color = i.color;
    o.params = i.params;
    o.uv = i.uv;
    o.stops = i.stops;
    o.gline = i.gline;
    o.rect = i.rect;
    return o;
}

// Rounded box SDF with per-corner radii (tl tr br bl), y pointing down.
fn sd_box(p: vec2<f32>, box: vec4<f32>, r: vec4<f32>) -> f32 {
    let half = box.zw * 0.5;
    let c = p - (box.xy + half);
    var rr = r.x;
    if (c.x >= 0.0) {
        rr = select(r.y, r.z, c.y >= 0.0);
    } else {
        rr = select(r.x, r.w, c.y >= 0.0);
    }
    rr = min(rr, min(half.x, half.y));
    let q = abs(c) - half + vec2<f32>(rr, rr);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - rr;
}

fn cover(d: f32) -> f32 {
    return clamp(0.5 - d, 0.0, 1.0);
}

fn stop(s: vec4<u32>, i: u32) -> vec4<f32> {
    return unpack4x8unorm(s[min(i, 3u)]);
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    let p = v.p;
    // clip: rectangle intersection with the innermost rounded clip
    let cbox = vec4<f32>(v.clip.xy, v.clip.zw - v.clip.xy);
    var a = cover(sd_box(p, cbox, v.clip_radii));
    if (a <= 0.0) {
        discard;
    }
    let kind = u32(v.params.x + 0.5);
    var col = v.color;
    if (kind == 0u) {
        // fill
        a = a * cover(sd_box(p, v.shape, v.radii));
    } else if (kind == 1u) {
        // linear gradient, `params.y` evenly spaced stops along gline
        a = a * cover(sd_box(p, v.shape, v.radii));
        let n = u32(v.params.y + 0.5);
        let d = v.gline.zw - v.gline.xy;
        let t = clamp(dot(p - v.gline.xy, d) / max(dot(d, d), 0.0001), 0.0, 1.0);
        let f = t * f32(n - 1u);
        let k = min(u32(floor(f)), n - 2u);
        col = mix(stop(v.stops, k), stop(v.stops, k + 1u), f - f32(k));
        col.a = col.a * v.color.a;
    } else if (kind == 2u) {
        // stroke of width params.y inside the shape
        let d = sd_box(p, v.shape, v.radii);
        a = a * clamp(cover(d) - cover(d + v.params.y), 0.0, 1.0);
    } else if (kind == 3u) {
        // soft shadow: blur params.y around the (offset) shape
        let d = sd_box(p, v.shape, v.radii);
        let b = max(v.params.y, 0.5);
        a = a * (1.0 - smoothstep(-b * 0.5, b * 0.5, d));
    } else if (kind == 4u) {
        // glyph from the coverage atlas; params.y = italic shear factor, params.z = baseline
        let row = floor(p.y);
        let shear = round((v.params.z - row) * v.params.y);
        let local = vec2<f32>(floor(p.x) - shear, row) - v.shape.xy;
        if (local.x < 0.0 || local.y < 0.0 || local.x >= v.shape.z || local.y >= v.shape.w) {
            discard;
        }
        a = a * textureLoad(atlas, vec2<i32>(v.uv.xy + local), 0).r;
    } else if (kind == 5u) {
        // image: uv.xy dest origin, uv.zw dest size; params.y > 0 = tint with color
        a = a * cover(sd_box(p, v.shape, v.radii));
        let t = textureSampleLevel(image, samp, (p - v.uv.xy) / v.uv.zw, 0.0);
        if (v.params.y > 0.5) {
            col = vec4<f32>(v.color.rgb, v.color.a * t.a);
        } else {
            col = vec4<f32>(t.rgb / max(t.a, 0.0001), t.a * v.color.a);
        }
    }
    let alpha = col.a * a;
    return vec4<f32>(col.rgb * alpha, alpha);
}
