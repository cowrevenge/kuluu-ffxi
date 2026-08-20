// D3m particle element shader.
//
// The mesh carries retail's texture-stage inputs collapsed onto the vertex colour: stage 0's
// diffuse D times stage 1's TEXTUREFACTOR F and its MODULATE gain, all computed CPU-side in
// particle_sim::vertex_color. What cannot be computed there is the texture argument T, which
// only exists in the sampler — so the stage-1 saturation belongs here, AFTER the texel
// multiply, not before it
// (research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:16-104).

#import bevy_pbr::forward_io::VertexOutput

struct ParticleUniform {
    // x = the premultiply the fixed-function blend state expects, mirroring
    // bevy_pbr::pbr_functions::premultiply_alpha. A custom fragment shader bypasses that
    // function, so an Add-mode particle that does not premultiply here erases the
    // background by (1 - a) instead of adding to it.
    params: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> data: ParticleUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var particle_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var particle_sampler: sampler;

const PREMULTIPLY_NONE: f32 = 0.0;
const PREMULTIPLY_ADD: f32 = 1.0;
const PREMULTIPLY_MULTIPLY: f32 = 2.0;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
#ifdef VERTEX_UVS_A
    var staged = textureSample(particle_texture, particle_sampler, in.uv);
#else
    var staged = vec4<f32>(1.0);
#endif
#ifdef VERTEX_COLORS
    staged = staged * in.color;
#endif
    let color = clamp(staged, vec4<f32>(0.0), vec4<f32>(1.0));

    if data.params.x == PREMULTIPLY_ADD {
        return vec4<f32>(color.rgb * color.a, 0.0);
    }
    if data.params.x == PREMULTIPLY_MULTIPLY {
        return vec4<f32>(color.rgb * color.a, color.a);
    }
    return color;
}
