#![cfg(not(target_arch = "wasm32"))]

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use crate::dat_d3m::D3mBlendMode;

// `ffxi_particle.wgsl`'s `PREMULTIPLY_*`: which premultiply the blend state this alpha mode
// resolves to expects. Bevy applies these inside `pbr_functions::premultiply_alpha`, which
// only the StandardMaterial shader calls, so a custom fragment shader owes them itself.
const PREMULTIPLY_NONE: f32 = 0.0;
const PREMULTIPLY_ADD: f32 = 1.0;
const PREMULTIPLY_MULTIPLY: f32 = 2.0;

#[derive(Clone, Debug, ShaderType)]
pub struct ParticleUniform {
    pub params: Vec4,
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub struct FfxiParticleMaterial {
    #[uniform(0)]
    pub data: ParticleUniform,

    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,

    pub alpha_mode: AlphaMode,
}

impl FfxiParticleMaterial {
    pub fn new(blend: D3mBlendMode, texture: Option<Handle<Image>>) -> Self {
        let alpha_mode = blend.alpha_mode();
        let premultiply = match alpha_mode {
            AlphaMode::Add => PREMULTIPLY_ADD,
            AlphaMode::Multiply => PREMULTIPLY_MULTIPLY,
            _ => PREMULTIPLY_NONE,
        };
        Self {
            data: ParticleUniform {
                params: Vec4::new(premultiply, 0.0, 0.0, 0.0),
            },
            texture,
            alpha_mode,
        }
    }
}

impl Material for FfxiParticleMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://kuluu_render/ffxi_particle.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // CMoElem::PrepDX sets D3DRS_CULLMODE to D3DCULL_NONE for every particle element
        // (research/XIClient/.../CMoElem.cpp:537).
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct FfxiParticleMaterialPlugin;

impl Plugin for FfxiParticleMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "ffxi_particle.wgsl");
        app.add_plugins(MaterialPlugin::<FfxiParticleMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bevy resolves Add and Premultiplied to one BlendState and distinguishes them only by
    // what the fragment shader emits, so the premultiply selector has to track alpha_mode.
    #[test]
    fn premultiply_selector_tracks_the_blend_mode() {
        let sel = |b| FfxiParticleMaterial::new(b, None).data.params.x;
        assert_eq!(sel(D3mBlendMode::Additive), PREMULTIPLY_ADD);
        assert_eq!(sel(D3mBlendMode::Subtractive), PREMULTIPLY_MULTIPLY);
        assert_eq!(sel(D3mBlendMode::Blended), PREMULTIPLY_NONE);
    }

    #[test]
    fn alpha_mode_matches_the_d3m_blend_mode() {
        for blend in [
            D3mBlendMode::Additive,
            D3mBlendMode::Blended,
            D3mBlendMode::Subtractive,
        ] {
            assert_eq!(
                FfxiParticleMaterial::new(blend, None).alpha_mode,
                blend.alpha_mode()
            );
        }
    }
}
