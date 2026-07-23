// FFXI faithful zone/scenery shader — an UNSKINNED port of the character
// shader (skinned_ffxi.wgsl), reproducing FFXI's zone-mesh lighting model
// (cross-referenced against research/xim's poc/gl/XimShader.kt:179-187):
//
//   out = 2 * (vertexColor * (ambient + 2 directional + 4 point)) * texel
//
// The baked per-vertex colour is the PRIMARY illumination. FFXI stores it as
// byte/128 (dat_mmb.rs), so a baked value of 255 maps to ~2.0 — "overbright".
// That overbright vertex colour, times the ambient floor, times the final 2x
// boost, is what makes lamps/braziers glow at night with no dynamic light.
//
// Bevy's StandardMaterial (which this replaces for zone meshes) treats vertex
// colour as albedo clamped to [0,1] and requires a live light to be visible,
// so at night the whole scene — lamps included — went dark. This shader keeps
// the overbright term and the 2x compositing, matching the actor path.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::{position_world_to_clip, position_world_to_view},
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
    shadows,
}

// FFXI point-light falloff, applied to Bevy's clustered point lights (the
// FaithfulZoneLight PointLights). Mirrors zone_point_lights.rs: peak factor
// 1/const at the base, quad term K/range². Kept in lockstep with
// SCENE_LIGHT_CONST_ATTEN / SCENE_LIGHT_FALLOFF_K there.
const FFXI_POINT_CONST_ATTEN: f32 = 1.0;
const FFXI_POINT_FALLOFF_K: f32 = 3.0;
// Bevy encodes clusterable color as `color·(intensity/4π)` (light.rs:1295 +
// :534). Our FaithfulZoneLight intensity is FAITHFUL_LIGHT_INTENSITY·peak·gate
// (zone_point_lights.rs), so multiplying by 4π/FAITHFUL_LIGHT_INTENSITY recovers
// the FFXI-native colour magnitude (~peak·gate) the vertex-lit model expects.
// 25000 = FAITHFUL_LIGHT_INTENSITY.
const FFXI_CLUSTER_COLOR_SCALE: f32 = 12.566370614 / 25000.0;

// Distance fog. FFXI fades distant terrain/water into the horizon backdrop
// (the weather-DAT `fog_landscape` colour, also painted as ClearColor). Bevy
// sets the `DistanceFog` on the camera (weather.rs), but this custom material
// bypasses the StandardMaterial fragment that would apply it — without this the
// far terrain falls straight to the void behind the sky dome. The `fog` binding
// (mesh_view_bindings group 0 @binding 13) and this def exist only when the
// view carries a DistanceFog (mesh.rs pushes the DISTANCE_FOG shaderdef).
#ifdef DISTANCE_FOG
#import bevy_pbr::fog as fog_fns
#endif

// Mirror of `FfxiLightingUniform` in skinned_ffxi_material.rs — field
// order/types must stay identical so AsBindGroup's std140 layout matches.
struct FfxiLighting {
    ambient: vec4<f32>,
    dir0_dir: vec4<f32>,
    dir0_color: vec4<f32>,
    dir1_dir: vec4<f32>,
    dir1_color: vec4<f32>,
    point_pos: array<vec4<f32>, 16>,
    point_color: array<vec4<f32>, 16>,
    point_atten: array<vec4<f32>, 16>,
    // x = elapsed seconds, y = wind strength, z/w reserved.
    time_params: vec4<f32>,
};

// Mirror of `FfxiMaterialFlags`. `flags.x` = has_texture (1.0 / 0.0);
// `flags.y` = blend mode (1.0 = translucent water/glass sub, emit real alpha);
// `flags.w` = alpha discard threshold (0.0 = no discard, e.g. opaque subs).
struct FfxiMaterialFlags {
    flags: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> lighting: FfxiLighting;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> material_flags: FfxiMaterialFlags;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> uv_offset: vec4<f32>;
// Per-mesh ToD tint: rgb is the cloud/sun-mesh color setter, w an alpha multiplier.
// White (1,1,1,1) for every non-cloud zone mesh, so this is a no-op there.
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> tint: vec4<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) color: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // Standard (unskinned) mesh placement: the MMB instance's world transform.
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world_position = world_from_local * vec4<f32>(v.position, 1.0);
    out.world_position = world_position.xyz;
    out.clip_position = position_world_to_clip(world_position.xyz);
    out.world_normal = normalize(mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index));
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

// Every FFXI zone point light (braziers/lamps) is a real Bevy PointLight, so
// Bevy's clustered forward lighting bins them spatially and this loops only the
// lights whose cluster covers the fragment — efficient even in the ~250-light
// zones, and free of the pop-in a nearest-N-to-viewer feed causes. FFXI falloff
// (not Bevy PBR) is applied so the look/brightness match the vertex-lit model.
fn clustered_point_irradiance(n: vec3<f32>, p: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    var rgb = vec3<f32>(0.0);
    let view_z = dot(
        vec4<f32>(
            view_bindings::view.view_from_world[0].z,
            view_bindings::view.view_from_world[1].z,
            view_bindings::view.view_from_world[2].z,
            view_bindings::view.view_from_world[3].z,
        ),
        vec4<f32>(p, 1.0),
    );
    let is_ortho = view_bindings::view.clip_from_view[3].w == 1.0;
    let cluster_index = clustering::view_fragment_cluster_index(frag_coord, view_z, is_ortho);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    for (var i = ranges.first_point_light_index_offset;
            i < ranges.first_spot_light_index_offset; i = i + 1u) {
        let light_id = clustering::get_clusterable_object_id(i);
        let lo = view_bindings::clustered_lights.data[light_id];
        let inv_sq_range = lo.color_inverse_square_range.w;
        if (inv_sq_range <= 0.0) { continue; }
        let range = inverseSqrt(inv_sq_range);
        let to_light = lo.position_radius.xyz - p;
        let dist = length(to_light);
        if (dist > range) { continue; }
        let color = lo.color_inverse_square_range.rgb * FFXI_CLUSTER_COLOR_SCALE;
        // Match zone_point_lights.rs: quad = K/range², const term, windowed to 0
        // at the range edge (no hard cutoff seam).
        let denom = FFXI_POINT_CONST_ATTEN + FFXI_POINT_FALLOFF_K * inv_sq_range * dist * dist;
        let inv = select(0.0, 1.0 / denom, denom > 0.0);
        let t = dist / range;
        let window = 1.0 - t * t;
        let nl = max(dot(n, to_light / max(dist, 1e-5)), 0.0);
        rgb += nl * inv * window * window * color;
    }
    return rgb;
}

// Pure scene light (ambient sky fill + 2 directional + clustered point lights),
// no vertex colour folded in — the caller multiplies by vertex colour, matching
// skinned_ffxi.wgsl::scene_irradiance.
fn scene_irradiance(n: vec3<f32>, p: vec3<f32>, sun_scale: f32, frag_coord: vec2<f32>) -> vec3<f32> {
    var rgb = lighting.ambient.rgb;
    let nl0 = max(dot(n, -lighting.dir0_dir.xyz), 0.0);
    rgb += sun_scale * nl0 * lighting.dir0_color.rgb * lighting.dir0_color.w;
    let nl1 = max(dot(n, -lighting.dir1_dir.xyz), 0.0);
    rgb += nl1 * lighting.dir1_color.rgb * lighting.dir1_color.w;
    rgb += clustered_point_irradiance(n, p, frag_coord);
    return rgb;
}

// Directional cast-shadow factor for the sun term (dir0). Bevy owns the real
// directional lights + cascade shadow maps at group(0); take the min shadow
// factor over the shadow-enabled ones (1 = lit, 0 = occluded). Mirrors the
// actor shader's sun_shadow_factor. No shadow-enabled light → returns 1.0.
fn sun_shadow_factor(world_pos: vec3<f32>, world_normal: vec3<f32>, frag_coord_xy: vec2<f32>) -> f32 {
    let view_z = position_world_to_view(world_pos).z;
    let n = view_bindings::lights.n_directional_lights;
    var factor = 1.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let lflags = view_bindings::lights.directional_lights[i].flags;
        if ((lflags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
            continue;
        }
        factor = min(factor, shadows::fetch_directional_shadow(
            i, vec4<f32>(world_pos, 1.0), world_normal, view_z, frag_coord_xy));
    }
    return factor;
}

// Fade a lit fragment toward the fog colour by view distance. Scattering is
// left at zero (the weather DAT drives a flat horizon colour, not sun-inscatter
// fog), so this is the plain distance blend Bevy's `apply_fog` does. No-op when
// the view has no DistanceFog (the whole body compiles out).
fn apply_distance_fog(color: vec4<f32>, world_pos: vec3<f32>) -> vec4<f32> {
#ifdef DISTANCE_FOG
    let fog_params = view_bindings::fog;
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
    let has_texture = material_flags.flags.x > 0.5;
    var texel = vec4<f32>(1.0);
    if (has_texture) {
        texel = textureSample(base_tex, base_samp, in.uv + uv_offset.xy);
    }
    // XIM `coloredPixel.a = vertexColor.a * texel.a` (XIM's 4·(va/255)·(ta/255)
    // matches our /128 vertex alpha × remapped texel alpha). Vertex alpha is a
    // second alpha-clip layer FFXI leans on for river/water edges, so fold it
    // into the cutout discard — not just the blend output. Opaque subs carry
    // flags.w = 0, so the test never fires and ground/walls stay solid. A custom
    // fragment bypasses Bevy's built-in mask handling, so do the test manually.
    let combined_a = clamp(in.color.a, 0.0, 1.0) * texel.a;
    if (combined_a < material_flags.flags.w) {
        discard;
    }
    let n = normalize(in.world_normal);
    // Cast-shadow attenuation on the sun term only (ambient/point fill the rest,
    // so a shadowed fragment darkens without crushing to black).
    let sun = sun_shadow_factor(in.world_position, n, in.clip_position.xy);
    // XIM's `2 * vertexColor * texel`, with vertexColor modulating the scene
    // light. Vertex colour is overbright (can exceed 1) — do NOT clamp it.
    let lit = scene_irradiance(n, in.world_position, sun, in.clip_position.xy) * in.color.rgb;
    // research/xim ParticleGeneratorParser.kt:431-434: ToD color.rgb is a setter folded
    // over the lit texel; color multiplier (.w) scales the emitted alpha.
    let rgb = 2.0 * lit * texel.rgb * tint.rgb;
    // 0x8000 subs (water/glass) emit the blended alpha; everything else opaque.
    var out_alpha = 1.0;
    if (material_flags.flags.y > 0.5) {
        out_alpha = combined_a;
    }
    var out_color = vec4<f32>(rgb, out_alpha * tint.w);
    out_color = apply_distance_fog(out_color, in.world_position);
    return out_color;
}
