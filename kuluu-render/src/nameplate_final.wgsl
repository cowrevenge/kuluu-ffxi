// Nameplates, drawn as a final pass inside the operator view (see
// nameplate_final_pass.rs for the scheduling story). The vertex layout is the
// bevy_mesh unit Rectangle verbatim (src/primitives/dim2.rs): positions are the
// z=0 square corners in order (+y,-y), and uvs run (1,0),(0,0),(0,1),(1,1) —
// replicate both exactly or every plate flips/mirrors relative to retail.

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

// Per-plate data, rewritten by the CPU each frame before the pass. Byte layout
// (80 total): model mat4 at 0, fade alpha f32 at 64, pad to 80.
struct PlateUniform {
    model: mat4x4<f32>,
    fade_alpha: f32,
};

// Per-view data for this frame's run of the pass: the clip matrix of the view
// currently being drawn (the operator camera's), plus its projection near
// plane. Byte layout: mat4 @ 0, near f32 @ 64.
struct ViewUniform {
    clip_from_world: mat4x4<f32>,
    near: f32,
};

@group(0) @binding(0) var<uniform> plate: PlateUniform;
@group(0) @binding(1) var<uniform> view_u: ViewUniform;
@group(0) @binding(2) var plate_tex: texture_2d<f32>;
@group(0) @binding(3) var plate_smp: sampler;

// Scene depth of the view this pass draws into. The single-sample variant tests
// it as a HARDWARE attachment instead and never touches these bindings (the
// group is simply left unbound); they exist so one pipeline layout serves both
// variants. Only referenced when MANUAL_DEPTH_TEST is compiled in, i.e. MSAA on:
// the multi-sample depth buffer cannot sit beside the 1-sample processed color
// image in one pass, so textureGather reads this pixel's four sub-sampled scene
// depths (same mechanism bevy's depth-prepass mesh shaders use).
#ifdef MANUAL_DEPTH_TEST
@group(1) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(1) @binding(1) var depth_smp: sampler;
#endif

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
    // View distance d = clip.w of this point (> 0 in front of the camera).
    // Perspective-correct delivery makes it exact at every pixel of this flat
    // camera-facing quad — for any planar patch, 1/d is affine in screen space,
    // so interpolating d and dividing by it recovers true per-pixel distance.
    // The depth value the main pass stores is near/d (bevy's reversed-Z infinite
    // projection: near -> 1, far -> 0, closer = LARGER); a pre-computed ratio
    // would NOT survive interpolation (~12% off mid-quad), so only d travels.
    @location(2) view_dist: f32,
};

@vertex
fn vs(v: VsIn) -> VsOut {
    let world = plate.model * vec4<f32>(v.position.xyz, 1.0);
    var out: VsOut;
    out.clip = view_u.clip_from_world * world;
    out.uv = v.uv;
    out.alpha = plate.fade_alpha;
    out.view_dist = out.clip.w;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    #ifdef MANUAL_DEPTH_TEST
    // Behind the camera (clip.w <= 0): nothing meaningful to compare against.
    // The billboard system already gates these upstream — this is the belt.
    if (in.clip.w <= 0.0) {
        discard;
    }
    // The four sub-sampled scene depths of this pixel: same encoding as the
    // plate's own stored value — closer = LARGER (near/d). Any sub-sample nearer
    // than the plate means opaque geometry occupies part of this pixel, so the
    // plate hides behind it — mirroring the hardware GreaterEqual test the
    // attachment variant uses when MSAA is off. Compare distances instead: both
    // sides divide by the same near (cancels), and gathered values can be 0 at
    // the far plane, where division would blow up.
    let d_scene = textureGather(scene_depth, depth_smp, in.uv);
    if (all(d_scene > vec4<f32>(1e-6))) {
        let scene_dist = view_u.near / d_scene;          // geometry distance per sub-sample
        if (any(scene_dist < in.view_dist)) {            // nearer than the plate -> occludes
            discard;
        }
    }
    #endif

    // Replicates core_3d's PBR unlit + AlphaMode::Premultiplied pixel math over the
    // processed scene (see module docs in nameplate_final_pass.rs): sample gives the
    // linearized premultiplied texel (raster is Rgba8UnormSrgb, so wgpu converts on
    // sample; premultiplied BEFORE mips per kuluu-zxxb), then color and coverage
    // scale together by the fade alpha — so a target pulse can't turn additive.
    let texel = textureSample(plate_tex, plate_smp, in.uv);
    return vec4<f32>(texel.rgb * in.alpha, texel.a * in.alpha);
}
