// Screen-space lens flare — the FFXI-faithful sun glare.
//
// The mesh is a unit Rectangle placed in front of the camera and scaled to
// (over)fill the frustum, so its UV [0,1]² maps to the screen. Unlike the old
// CPU path, the sun's screen position is projected HERE, against the live view
// matrix the renderer is using this frame — so the flare can't lag the camera.
// Occlusion is a CPU raycast against the zone collision BVH, fed in as
// flare_params.w (lens_flare.rs SunOcclusion), so the flare fades behind
// terrain without needing a depth prepass.
//
// Additive blend: where the flare contributes nothing the fragment is black
// (adds zero), so the quad can cover the whole screen cheaply.
//
// The chain is data-driven off the zone's lf0x lens-flare sprite sheet: each element
// is an additive textured quad placed along the sun→screen-centre axis at
// sun*(1-offset)+opposite*offset, sized viewport/32 (research/xim
// ZoneDrawer.kt:231-236). Retail draws no flare where a zone ships no sheet, so
// lens_flare.rs hides the quad instead of substituting anything.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view

const MAX_FLARE_ELEMENTS: u32 = 16u;

struct LensFlareUniform {
    // xyz = normalized world-space sun direction, w = intensity [0,1].
    sun_dir_intensity: vec4<f32>,
    // rgb = flare tint, a = unused.
    tint: vec4<f32>,
    // x = element count, yz unused, w = sun visibility [0,1] (CPU BVH raycast).
    flare_params: vec4<f32>,
    // x = per-element offset fraction along sun->opposite.
    offsets: array<vec4<f32>, 16>,
    // (u0,v0,u1,v1) sub-rect of each element in the lf0x texture.
    frame_uv: array<vec4<f32>, 16>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> data: LensFlareUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var flare_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var flare_samp: sampler;

// Distance used to synthesize a world point in the sun's direction. Matched to
// the skybox radius (sun_moon::SKY_RADIUS, pinned by a guard test in
// lens_flare.rs) so the projected flare sits exactly on the sun disc.
const SUN_SKY_RADIUS: f32 = 4000.0;

fn fully_transparent() -> vec4<f32> {
    // Premultiplied-alpha "Add" blend (Bevy maps AlphaMode::Add to
    // BLEND_PREMULTIPLIED_ALPHA = src·1 + dst·(1−src.a)). Output alpha MUST be 0
    // so the destination is preserved and we add nothing.
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let intensity = data.sun_dir_intensity.w;
    if (intensity <= 0.0) {
        return fully_transparent();
    }

    // --- Project the sun to screen space against the live view matrix. ---
    let sun_dir = data.sun_dir_intensity.xyz;
    let sun_world = view.world_position + sun_dir * SUN_SKY_RADIUS;
    let sun_clip = view.clip_from_world * vec4<f32>(sun_world, 1.0);
    if (sun_clip.w <= 0.0) {
        return fully_transparent(); // sun behind the camera
    }
    let sun_ndc = sun_clip.xy / sun_clip.w;
    var sun = sun_ndc * 0.5 + vec2<f32>(0.5);
    sun.y = 1.0 - sun.y; // NDC (y up) → UV (y down), matching in.uv

    let aspect = view.viewport.z / max(view.viewport.w, 1.0);

    let visibility = data.flare_params.w;
    if (visibility <= 0.0) {
        return fully_transparent();
    }

    // Screen UV from the framebuffer coord, NOT in.uv: the quad is oversized by
    // FLARE_OVERSCAN, so its [0,1] UV spills past the frustum and would drift the
    // flare off the sun by overscan·(uv−0.5). frag coord ÷ viewport is the true
    // screen position, matching the projected `sun` regardless of overscan.
    let uv = (in.position.xy - view.viewport.xy) / max(view.viewport.zw, vec2<f32>(1.0));
    let centre = vec2<f32>(0.5, 0.5);
    let tint = data.tint.rgb;

    var col = vec3<f32>(0.0);

    // --- Data-driven lf0x chain (research/xim ZoneDrawer.kt:233-236). ---
    // Each lens-flare mesh is an additive textured quad placed at
    // sun*(1-offset) + opposite*offset along the sun->screen-centre axis (opposite =
    // sun + 2*to_centre), sized from its own quad geometry. Intensity rides the
    // raycast occlusion `visibility`.
    let count = u32(data.flare_params.x);
    let to_centre = centre - sun;
    let opposite = sun + to_centre * 2.0;
    for (var i = 0u; i < count && i < MAX_FLARE_ELEMENTS; i = i + 1u) {
        let offset = data.offsets[i].x;
        let pos = sun * (1.0 - offset) + opposite * offset;
        // Per-element half-extent, already in screen-UV (lens_flare.rs divides the
        // mesh quad by LENS_FLARE_SCREEN_UNITS, retail's screen/32 draw scale). Each
        // axis divides by its own screen dimension, so the flare stretches with the
        // aspect ratio exactly as retail's pixel-space ortho draw does.
        let half = data.offsets[i].yz;
        let local = (uv - pos) / half; // [-1,1] inside the quad
        // Clamp + mask rather than `continue`, so the textureSample stays in
        // uniform control flow (WGSL requires implicit-LOD sampling there).
        let inside = step(abs(local.x), 1.0) * step(abs(local.y), 1.0);
        let quad_uv = clamp(local * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
        let f = data.frame_uv[i];
        let suv = vec2<f32>(mix(f.x, f.z, quad_uv.x), mix(f.y, f.w, quad_uv.y));
        let texel = textureSample(flare_tex, flare_samp, suv);
        col += tint * texel.rgb * texel.a * inside;
    }

    // Fade toward the screen edges — a real flare is strongest with the sun framed,
    // washing out as it leaves view. Premultiplied additive: rgb is the light to add.
    let edge = 1.0 - smoothstep(0.35, 0.75, length((sun - centre) * vec2<f32>(aspect, 1.0)));
    return vec4<f32>(col * intensity * visibility * edge, 0.0);
}
