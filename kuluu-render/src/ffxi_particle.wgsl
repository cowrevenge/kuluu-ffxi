// D3m particle element shader.
//
// The mesh carries retail's texture-stage inputs collapsed onto the vertex colour: stage 0's
// diffuse D times stage 1's TEXTUREFACTOR F and its MODULATE gain, all computed CPU-side in
// particle_sim::vertex_color. What cannot be computed there is the texture argument T, which
// only exists in the sampler — so the stage-1 saturation belongs here, AFTER the texel
// multiply, not before it
// (research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:16-104).

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings as view_bindings
#import bevy_pbr::mesh_view_types
#ifdef DISTANCE_FOG
#import bevy_pbr::fog as fog_fns
#endif

struct ParticleUniform {
    // x = the premultiply the fixed-function blend state expects, mirroring
    // bevy_pbr::pbr_functions::premultiply_alpha. A custom fragment shader bypasses that
    // function, so an Add-mode particle that does not premultiply here erases the
    // background by (1 - a) instead of adding to it.
    // y = the element's fog selector (FOG_*), from ffxi_particle_material.rs ParticleFog.
    params: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> data: ParticleUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var particle_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var particle_sampler: sampler;

const PREMULTIPLY_NONE: f32 = 0.0;
const PREMULTIPLY_ADD: f32 = 1.0;
const PREMULTIPLY_MULTIPLY: f32 = 2.0;

const FOG_OFF: f32 = 0.0;
const FOG_ZONE: f32 = 1.0;
const FOG_BLACK: f32 = 2.0;

// research/XIClient/src/XIClient/source/World/Generator/Effects/CMoElem.cpp CMoElem::PrepDX —
// a fogged element takes the area's linear fog like terrain (zone_ffxi.wgsl apply_distance_fog),
// with the colour forced to black for FOG_BLACK. Fixed-function fog lands on the fragment
// before the blend, so this runs before the premultiply below.
fn apply_element_fog(color: vec4<f32>, world_pos: vec3<f32>, selector: f32) -> vec4<f32> {
#ifdef DISTANCE_FOG
    if selector == FOG_OFF {
        return color;
    }
    var fog_params = view_bindings::fog;
    if selector == FOG_BLACK {
        fog_params.base_color = vec4<f32>(0.0, 0.0, 0.0, fog_params.base_color.a);
    }
    let dist = length(world_pos - view_bindings::view.world_position);
    let scattering = vec3<f32>(0.0);
    if (fog_params.mode == mesh_view_types::FOG_MODE_LINEAR) {
        return fog_fns::linear_fog(fog_params, color, dist, scattering);
    } else if (fog_params.mode == mesh_view_types::FOG_MODE_EXPONENTIAL) {
        return fog_fns::exponential_fog(fog_params, color, dist, scattering);
    } else if (fog_params.mode == mesh_view_types::FOG_MODE_EXPONENTIAL_SQUARED) {
        return fog_fns::exponential_squared_fog(fog_params, color, dist, scattering);
    } else if (fog_params.mode == mesh_view_types::FOG_MODE_ATMOSPHERIC) {
        return fog_fns::atmospheric_fog(fog_params, color, dist, scattering);
    }
#endif
    return color;
}

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
    let color = apply_element_fog(
        clamp(staged, vec4<f32>(0.0), vec4<f32>(1.0)),
        in.world_position.xyz,
        data.params.y,
    );

    if data.params.x == PREMULTIPLY_ADD {
        return vec4<f32>(color.rgb * color.a, 0.0);
    }
    if data.params.x == PREMULTIPLY_MULTIPLY {
        return vec4<f32>(color.rgb * color.a, color.a);
    }
    return color;
}
