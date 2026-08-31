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
// currently being drawn (the operator camera's).
struct ViewUniform {
    clip_from_world: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> plate: PlateUniform;
@group(0) @binding(1) var<uniform> view_u: ViewUniform;
@group(0) @binding(2) var plate_tex: texture_2d<f32>;
@group(0) @binding(3) var plate_smp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
};

@vertex
fn vs(v: VsIn) -> VsOut {
    let world = plate.model * vec4<f32>(v.position.xyz, 1.0);
    var out: VsOut;
    out.clip = view_u.clip_from_world * world;
    out.uv = v.uv;
    out.alpha = plate.fade_alpha;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // TEMP DEBUG (magenta probe) — visibility hunt. Bypasses the plate texture
    // entirely: constant magenta × real fade alpha, premultiplied to match the
    // ONE / ONE_MINUS_SRC_ALPHA pipeline blend.
    //   rectangles over heads  → target/placement/blend all fine; TEXTURE/SAMPLER is the culprit
    //   nothing visible        → placement or wrong write target (A/B side, 100%-scale path)
    // REMOVE this probe and restore the textureSample version when done.
    return vec4<f32>(vec3<f32>(1.0, 0.0, 1.0) * in.alpha, in.alpha);
}
