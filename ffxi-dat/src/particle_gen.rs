use crate::{DatError, Result};

// research/xim ParticleGeneratorParser.kt + ParticleInitializers.kt + ParticleKeyFrameSection.kt
//
// A 0x05 Generator chunk whose StandardParticleSetup (sec2 op 0x01) links data-type 0x0B is a
// particle emitter. The generator header and four section-offset words sit in the chunk body
// (which already excludes the 16-byte chunk header, so a ByteReader `sectionStart + X` maps to
// body index `X - 0x10`):
//   body[0x00] u16  attachFlags           (XIM reads these two via offsetFromDataStart, i.e. body
//   body[0x02] u16  additionalAttachFlags  index 0 — ParticleGeneratorParser.kt:21-33)
//   body[0x64] u16  emissionVariance
//   body[0x66] u16  framesPerEmission - 1
//   body[0x68] u8   particlesPerEmission
//   body[0x69] u8   genFlags
//   body[0x70..0x80] four u32 section offsets (section data at value - 0x10)
// Each section is a stream of opcodeConfig u32s: opcode = cfg & 0xFF, size_words = (cfg>>8)&0x1F,
// allocationOffset = cfg>>0xD; the block is size_words*4 bytes; a 0 opcode/size terminates.
// Only section 2 (particle initializers) is needed for the visible stream.
const HEADER_LEN: usize = 0x80;
// research/xim ParticleGeneratorSettings.kt:187 — the StandardParticleSetup linked_data_type
// (setup byte payload+29) selects the particle's mesh source: 0x0B StaticMesh (a D3M billboard),
// 0x0E SpriteSheet (a 0x21 flipbook quad). 0x57 Null / 0x47 PointLight and any other value are
// non-visual particle types and are rejected (parse returns None).
const LINKED_DATA_STATIC_MESH: u8 = 0x0B;
const LINKED_DATA_SPRITE_SHEET: u8 = 0x0E;

// research/xim ParticleGeneratorSettings.kt:187 (mesh source) + Particle.kt:72 (the per-particle
// spriteSheetIndex cursor) + ParticleUpdaters.kt:196-211 (SpriteSheetFrameUpdater advances it over
// life). StaticMesh binds a D3M; SpriteSheet binds a 0x21 sprite-sheet whose frames flipbook
// across the particle's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleMeshKind {
    #[default]
    StaticMesh,
    SpriteSheet,
}
// research/xim ParticleGeneratorSettings.kt:154 `enum class AttachType(val flag: Int)` — which
// actor (and whose facing) a generator's emission origin is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachType {
    #[default]
    None,
    SourceActor,
    TargetActor,
    SourceToTargetBasis,
    TargetActorSourceFacing,
    SourceActorTargetFacing,
    TargetToSourceBasis,
    SourceActorWeapon,
    ZoneActorA,
    ZoneActorB,
    ZoneActorC,
    Sun,
    Moon,
}

impl AttachType {
    pub fn from_flag(flag: u16) -> Option<Self> {
        Some(match flag {
            0x0 => Self::None,
            0x1 => Self::SourceActor,
            0x2 => Self::TargetActor,
            0x3 => Self::SourceToTargetBasis,
            0x4 => Self::TargetActorSourceFacing,
            0x5 => Self::SourceActorTargetFacing,
            0x6 => Self::TargetToSourceBasis,
            0x9 => Self::SourceActorWeapon,
            0xA => Self::ZoneActorA,
            0xB => Self::ZoneActorB,
            0xC => Self::ZoneActorC,
            0xE => Self::Sun,
            0xF => Self::Moon,
            _ => return None,
        })
    }
}

// research/xim ParticleGeneratorParser.kt:23-33 — attachFlags bit layout, then
// additionalAttachFlags bit 0x0001 = attachSourceOriented.
const ATTACH_TYPE_MASK: u16 = 0x000F;
const ATTACH_JOINT0_MASK: u16 = 0x03F0;
const ATTACH_JOINT0_SHIFT: u32 = 4;
const ATTACH_JOINT1_MASK: u16 = 0xFC00;
const ATTACH_JOINT1_SHIFT: u32 = 10;
const ATTACH_SOURCE_ORIENTED: u16 = 0x0001;

// research/xim ParticleInitializers.kt:105-116 — the StandardParticleSetup renderStateFlags u16
// sits directly after the billboard flags. Bit 0x1000 (`ignoreTextureAlpha`) is the same bit
// retail tests as `field_10C & 0x10000000` to pick the D3m element's texture-stage table.
// research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:363-370
const RENDER_STATE_IGNORE_TEXTURE_ALPHA: u16 = 0x1000;

// research/xim ParticleGeneratorParser.kt:68-70 (genFlags at body[0x69]);
// continuous singleton + auto-run-at-model-ready semantics: Actor.kt:724-734.
const GEN_FLAG_CONTINUOUS: u8 = 0x04;
const GEN_FLAG_AUTO_RUN: u8 = 0x10;

// Vana'diel's elemental week (research/xim EnvironmentManager.kt DayOfWeek) and the
// 12 moon-phase buckets the 0x45/0x4F celestial opcodes index.
pub const DAYS_OF_WEEK: usize = 8;
pub const MOON_PHASES: usize = 12;
// RGBA — one time-of-day keyframe track per channel (0x60 r .. 0x63 a).
pub const TOD_COLOR_CHANNELS: usize = 4;

fn rgba_u8(b: &[u8], o: usize) -> [f32; 4] {
    std::array::from_fn(|i| b[o + i] as f32 / 255.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DatId(pub [u8; 4]);

impl DatId {
    fn from(b: &[u8], off: usize) -> Self {
        Self([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }
    fn is_zero(&self) -> bool {
        self.0 == [0, 0, 0, 0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleBlend {
    #[default]
    Additive,
    Blend,
    Subtract,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParticleGeneratorDef {
    pub frames_per_emission: f32,
    pub particles_per_emission: u32,
    pub emission_variance: f32,

    pub mesh_id: [u8; 4],
    pub mesh_kind: ParticleMeshKind,
    pub base_position: [f32; 3],
    pub max_life_frames: f32,
    pub camera_billboard: bool,

    pub continuous: bool,
    pub auto_run: bool,

    pub attach_type: AttachType,
    pub attach_joint_source: u8,
    pub attach_joint_target: u8,
    pub attach_source_oriented: bool,

    pub init_scale: [f32; 3],
    pub init_color: [f32; 4],
    pub init_velocity: [f32; 3],
    pub init_rotation: [f32; 3],
    pub blend: ParticleBlend,
    // The raw BlendFuncInitializer p0 (retail `field_16C & 0xFF`), kept alongside the collapsed
    // `blend` because the TEXTUREFACTOR-alpha promotion is keyed on byte 0x44 exactly.
    // research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:345-349
    pub blend_byte: u8,

    // Selects the D3m texture-stage table: set = NonZeroOneTSS (texture alpha ignored,
    // alpha = 4*D.a*F.a), clear = NonZeroTwoTSS (alpha = 8*D.a*T.a*F.a).
    // research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:16-104
    pub ignore_texture_alpha: bool,

    // Per-particle keyframe tracks referenced by DAT-id (resolved against the action's 0x19 chunks).
    pub scale_x_track: Option<[u8; 4]>,
    pub scale_y_track: Option<[u8; 4]>,
    pub alpha_track: Option<[u8; 4]>,

    // research/xim ParticleUpdaters.kt:289-317 DayOfWeekColorUpdater (0x4E, 8xRGBA) and
    // MoonPhaseColorUpdater (0x4F, 12xRGBA): indexed by day-of-week / moon-phase frame and
    // applied as a 2x modulate (Particle.kt:217-218). RGBA in 0..=1.
    pub day_of_week_color: Option<[[f32; 4]; DAYS_OF_WEEK]>,
    pub moon_phase_color: Option<[[f32; 4]; MOON_PHASES]>,

    // The time-of-day color curves: initializer 0x60..0x63 name a keyframe track per RGBA
    // channel, and section-3 ClockValueUpdater 0x3C..0x3F sample it at the Vana'diel day
    // fraction rather than the particle's life progress. This is how retail authors the
    // sun's dawn/noon/dusk ramp and the moon's daytime fade — 0x3F multiplies alpha, the
    // other three assign their channel.
    // research/xim ParticleGeneratorParser.kt:270-274,431-434
    pub tod_color_tracks: [Option<[u8; 4]>; TOD_COLOR_CHANNELS],
    pub tod_color_driven: [bool; TOD_COLOR_CHANNELS],

    // research/xim ParticleGeneratorParser.kt:444 MoonPhaseSpriteSheetUpdater (0x45): the
    // sprite-sheet frame is the current moon phase, not the particle's life progress.
    pub moon_phase_sprite: bool,

    // research/xim ParticleUpdaters.kt section-3 updaters (offset at body[0x78], same
    // sectionHeader+offset-0x10 convention as the setup section). TextureCoordinateUpdater
    // 0x27/0x28 carry the per-frame UV-translate velocity that scrolls the sprite/sheet
    // texture (cascade/moat water). VelocityAccelerator 0x03/0x06/0x09 read a Vector3f at
    // payload+0; only 0x03 (gravity) affects the visible arc. [0,0]/None = static.
    pub uv_scroll: [f32; 2],
    pub accel: Option<[f32; 3]>,
}

impl ParticleGeneratorDef {
    pub fn parse(body: &[u8]) -> Result<Option<Self>> {
        if body.len() < HEADER_LEN {
            return Err(DatError::TruncatedChunk {
                offset: 0,
                needed: HEADER_LEN,
                available: body.len(),
            });
        }

        let attach_flags = u16_le(body, 0x00);
        let additional_attach = u16_le(body, 0x02);
        let attach_type =
            AttachType::from_flag(attach_flags & ATTACH_TYPE_MASK).unwrap_or_default();
        let attach_joint_source =
            ((attach_flags & ATTACH_JOINT0_MASK) >> ATTACH_JOINT0_SHIFT) as u8;
        let attach_joint_target =
            ((attach_flags & ATTACH_JOINT1_MASK) >> ATTACH_JOINT1_SHIFT) as u8;
        let attach_source_oriented = additional_attach & ATTACH_SOURCE_ORIENTED != 0;

        let frames_per_emission = u16_le(body, 0x66) as f32 + 1.0;
        let particles_per_emission = (body[0x68] as u32).max(1);
        let emission_variance = u16_le(body, 0x64) as f32;
        let gen_flags = body[0x69];
        let continuous = gen_flags & GEN_FLAG_CONTINUOUS != 0;
        let auto_run = gen_flags & GEN_FLAG_AUTO_RUN != 0;

        // Section 2 = particle initializers.
        let sec2_raw = u32_le(body, 0x74) as usize;
        if sec2_raw < 0x10 || sec2_raw - 0x10 >= body.len() {
            return Ok(None);
        }
        let mut cursor = sec2_raw - 0x10;

        let mut mesh_id = [0u8; 4];
        let mut mesh_kind = ParticleMeshKind::StaticMesh;
        let mut base_position = [0.0f32; 3];
        let mut max_life_frames = 0.0f32;
        let mut camera_billboard = false;
        let mut is_particle = false;
        let mut init_scale = [1.0f32; 3];
        let mut init_color = [1.0f32; 4];
        let mut init_velocity = [0.0f32; 3];
        let mut init_rotation = [0.0f32; 3];
        let mut scale_x_track = None;
        let mut scale_y_track = None;
        let mut alpha_track = None;
        let mut blend = ParticleBlend::Additive;
        let mut blend_byte = 0u8;
        let mut ignore_texture_alpha = false;
        let mut tod_color_tracks: [Option<[u8; 4]>; TOD_COLOR_CHANNELS] =
            [None; TOD_COLOR_CHANNELS];

        while cursor + 4 <= body.len() {
            let cfg = u32_le(body, cursor);
            let opcode = (cfg & 0xFF) as u8;
            let size_words = ((cfg >> 8) & 0x1F) as usize;
            if opcode == 0x00 || size_words == 0 {
                break;
            }
            let block_len = size_words * 4;
            let payload = cursor + 4;
            if cursor + block_len > body.len() {
                break;
            }
            match opcode {
                0x01 if payload + 32 <= body.len() => {
                    let bb = u16_le(body, payload);
                    camera_billboard = bb & 0x0001 != 0 || bb & 0x00C0 == 0x00C0;
                    let render_state = u16_le(body, payload + 2);
                    ignore_texture_alpha = render_state & RENDER_STATE_IGNORE_TEXTURE_ALPHA != 0;
                    mesh_id = [
                        body[payload + 8],
                        body[payload + 9],
                        body[payload + 10],
                        body[payload + 11],
                    ];
                    base_position = [
                        f32_le(body, payload + 16),
                        f32_le(body, payload + 20),
                        f32_le(body, payload + 24),
                    ];
                    (is_particle, mesh_kind) = match body[payload + 29] {
                        LINKED_DATA_STATIC_MESH => (true, ParticleMeshKind::StaticMesh),
                        LINKED_DATA_SPRITE_SHEET => (true, ParticleMeshKind::SpriteSheet),
                        _ => (false, ParticleMeshKind::StaticMesh),
                    };
                    max_life_frames = u16_le(body, payload + 30) as f32;
                }
                0x02 if payload + 12 <= body.len() => {
                    init_velocity = [
                        f32_le(body, payload),
                        f32_le(body, payload + 4),
                        f32_le(body, payload + 8),
                    ];
                }
                0x09 if payload + 12 <= body.len() => {
                    init_rotation = [
                        f32_le(body, payload),
                        f32_le(body, payload + 4),
                        f32_le(body, payload + 8),
                    ];
                }
                0x0F if payload + 12 <= body.len() => {
                    init_scale = [
                        f32_le(body, payload),
                        f32_le(body, payload + 4),
                        f32_le(body, payload + 8),
                    ];
                }
                0x16 if payload + 4 <= body.len() => {
                    init_color = [
                        body[payload] as f32 / 255.0,
                        body[payload + 1] as f32 / 255.0,
                        body[payload + 2] as f32 / 255.0,
                        body[payload + 3] as f32 / 255.0,
                    ];
                }
                // KeyFrameValueSetup: opcode selects the target channel; the track id is at payload+4.
                0x27 if payload + 8 <= body.len() => scale_x_track = track_id(body, payload + 4),
                0x28 if payload + 8 <= body.len() => scale_y_track = track_id(body, payload + 4),
                0x2D if payload + 8 <= body.len() => alpha_track = track_id(body, payload + 4),
                // research/xim ParticleGeneratorParser.kt:270-274 — 0x60..0x63 are the same
                // KeyFrameValueSetup shape bound to the time-of-day color channels, read back by
                // the section-3 ClockValueUpdater 0x3C..0x3F.
                0x60..=0x63 if payload + 8 <= body.len() => {
                    tod_color_tracks[(opcode - 0x60) as usize] = track_id(body, payload + 4);
                }
                // BlendFuncInitializer: p0 @payload+0 — high nibble bit 0x01 = opaque, else low
                // nibble selects (0x8 additive, 0x4/0x6 alpha blend, 0x1/0x2 reverse-subtract).
                0x1E if payload < body.len() => {
                    let p0 = body[payload];
                    blend_byte = p0;
                    blend = if (p0 >> 4) & 0x01 != 0 {
                        ParticleBlend::Blend
                    } else {
                        match p0 & 0x0F {
                            0x8 => ParticleBlend::Additive,
                            0x1 | 0x2 => ParticleBlend::Subtract,
                            _ => ParticleBlend::Blend,
                        }
                    };
                }
                _ => {}
            }
            cursor += block_len;
        }

        if !is_particle {
            return Ok(None);
        }

        // Section 3 (body[0x78]) — per-frame updaters (same walk as
        // generator.rs::parse_cloud_generator). 0x27/0x28 TextureCoordinateUpdater UV
        // scroll; 0x03 VelocityAccelerator gravity (Vector3f at payload+0).
        let mut uv_scroll = [0.0f32; 2];
        let mut accel = None;
        let mut day_of_week_color = None;
        let mut moon_phase_color = None;
        let mut moon_phase_sprite = false;
        let mut tod_color_driven = [false; TOD_COLOR_CHANNELS];
        let sec3_raw = u32_le(body, 0x78) as usize;
        if sec3_raw >= 0x10 && sec3_raw - 0x10 < body.len() {
            let mut cursor = sec3_raw - 0x10;
            while cursor + 4 <= body.len() {
                let cfg = u32_le(body, cursor);
                let opcode = (cfg & 0xFF) as u8;
                let size_words = ((cfg >> 8) & 0x1F) as usize;
                if opcode == 0x00 || size_words == 0 {
                    break;
                }
                let block_len = size_words * 4;
                let payload = cursor + 4;
                if cursor + block_len > body.len() {
                    break;
                }
                match opcode {
                    0x27 if payload + 4 <= body.len() => uv_scroll[0] = f32_le(body, payload),
                    0x28 if payload + 4 <= body.len() => uv_scroll[1] = f32_le(body, payload),
                    0x03 if payload + 12 <= body.len() => {
                        accel = Some([
                            f32_le(body, payload),
                            f32_le(body, payload + 4),
                            f32_le(body, payload + 8),
                        ]);
                    }
                    // research/xim ParticleGeneratorParser.kt:431-434 ClockValueUpdater — these
                    // carry no payload; they mark which 0x60..0x63 track drives its channel.
                    0x3C..=0x3F => tod_color_driven[(opcode - 0x3C) as usize] = true,
                    // research/xim ParticleGeneratorParser.kt:444 MoonPhaseSpriteSheetUpdater.
                    0x45 => moon_phase_sprite = true,
                    // research/xim ParticleUpdaters.kt:289-301 DayOfWeekColorUpdater: expectZero32
                    // then 8 RGBA quads (u8x4, 0..=255). payload+0 is the zero u32.
                    0x4E if payload + 4 + 4 * DAYS_OF_WEEK <= body.len() => {
                        day_of_week_color =
                            Some(std::array::from_fn(|i| rgba_u8(body, payload + 4 + i * 4)));
                    }
                    // research/xim ParticleUpdaters.kt:304-316 MoonPhaseColorUpdater: same shape,
                    // 12 quads.
                    0x4F if payload + 4 + 4 * MOON_PHASES <= body.len() => {
                        moon_phase_color =
                            Some(std::array::from_fn(|i| rgba_u8(body, payload + 4 + i * 4)));
                    }
                    _ => {}
                }
                cursor += block_len;
            }
        }

        Ok(Some(Self {
            frames_per_emission,
            particles_per_emission,
            emission_variance,
            mesh_id,
            mesh_kind,
            base_position,
            max_life_frames,
            camera_billboard,
            continuous,
            auto_run,
            attach_type,
            attach_joint_source,
            attach_joint_target,
            attach_source_oriented,
            init_scale,
            init_color,
            init_velocity,
            init_rotation,
            blend,
            blend_byte,
            ignore_texture_alpha,
            scale_x_track,
            scale_y_track,
            alpha_track,
            day_of_week_color,
            moon_phase_color,
            tod_color_tracks,
            tod_color_driven,
            moon_phase_sprite,
            uv_scroll,
            accel,
        }))
    }

    pub fn is_singleton(&self) -> bool {
        self.max_life_frames == 0.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyFrameTrack {
    pub points: Vec<(f32, f32)>,
}

impl KeyFrameTrack {
    // research/xim ParticleKeyFrameValueSection.read: (time, value) f32 pairs from the chunk body,
    // terminated by an entry whose time == 1.0.
    pub fn parse(body: &[u8]) -> Self {
        let mut points = Vec::new();
        let mut o = 0;
        while o + 8 <= body.len() {
            let t = f32_le(body, o);
            let v = f32_le(body, o + 4);
            points.push((t, v));
            o += 8;
            if t >= 1.0 {
                break;
            }
        }
        Self { points }
    }

    pub fn sample(&self, progress: f32) -> f32 {
        self.sample_from(progress, None)
    }

    // research/xim ParticleKeyFrameData.getCurrentValue. `initial` overrides the value of the very
    // first keyframe when interpolating the opening segment (a ProgressValueUpdater seeds the curve
    // with the particle's initial channel value, e.g. its starting scale).
    pub fn sample_from(&self, progress: f32, initial: Option<f32>) -> f32 {
        match self.points.as_slice() {
            [] => 0.0,
            [single] => single.1,
            pts => {
                if progress >= 1.0 {
                    return pts.last().unwrap().1;
                }
                let next = pts
                    .iter()
                    .position(|&(t, _)| t > progress)
                    .unwrap_or(pts.len() - 1);
                let next = next.max(1);
                let (pt, pv) = pts[next - 1];
                let (nt, nv) = pts[next];
                let pv = match initial {
                    Some(i) if next - 1 == 0 => i,
                    _ => pv,
                };
                let span = nt - pt;
                if span.abs() < 1e-9 {
                    return pv;
                }
                let f = (progress - pt) / span;
                (1.0 - f) * pv + f * nv
            }
        }
    }
}

fn track_id(b: &[u8], off: usize) -> Option<[u8; 4]> {
    let id = DatId::from(b, off);
    (!id.is_zero()).then_some(id.0)
}

#[inline]
fn u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

#[inline]
fn u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[inline]
fn f32_le(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a generator body matching the real layout: header at 0x64, section-2 offset word at
    // body[0x74] (value = body_index + 0x10), then the initializer opcode stream.
    fn build(sec2: &[u8], frames_per_em: u16, ppe: u8) -> Vec<u8> {
        build_attached(sec2, frames_per_em, ppe, 0, 0)
    }

    fn build_attached(
        sec2: &[u8],
        frames_per_em: u16,
        ppe: u8,
        attach_flags: u16,
        additional_attach: u16,
    ) -> Vec<u8> {
        let mut body = vec![0u8; HEADER_LEN];
        body[0x00..0x02].copy_from_slice(&attach_flags.to_le_bytes());
        body[0x02..0x04].copy_from_slice(&additional_attach.to_le_bytes());
        body[0x66..0x68].copy_from_slice(&(frames_per_em - 1).to_le_bytes());
        body[0x68] = ppe;
        let sec2_body_index = HEADER_LEN;
        body[0x74..0x78].copy_from_slice(&((sec2_body_index + 0x10) as u32).to_le_bytes());
        body.extend_from_slice(sec2);
        body
    }

    fn op(opcode: u8, size_words: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![opcode, size_words, 0, 0];
        v.extend_from_slice(payload);
        v.resize(size_words as usize * 4, 0);
        v
    }

    #[test]
    fn parses_particle_generator_header_and_setup() {
        let mut setup = op(0x01, 12, &[]);
        // billboard XYZ
        setup[4] = 0x01;
        // mesh id at payload+8 (payload = cursor+4 = setup index 4)
        setup[4 + 8..4 + 12].copy_from_slice(b"kir1");
        // base position y at payload+20
        setup[4 + 20..4 + 24].copy_from_slice(&0.2f32.to_le_bytes());
        // linked-data type at payload+29 = particle; max life u16 at payload+30
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        setup[4 + 30..4 + 32].copy_from_slice(&36u16.to_le_bytes());

        let mut sec2 = setup;
        sec2.extend(op(0x0F, 4, &{
            let mut p = Vec::new();
            p.extend_from_slice(&0.05f32.to_le_bytes());
            p.extend_from_slice(&0.05f32.to_le_bytes());
            p.extend_from_slice(&1.0f32.to_le_bytes());
            p
        }));
        sec2.extend(op(0x16, 2, &[46, 46, 158, 255]));
        sec2.extend(op(0x02, 4, &{
            let mut p = Vec::new();
            p.extend_from_slice(&0.0f32.to_le_bytes());
            p.extend_from_slice(&(-0.005f32).to_le_bytes());
            p.extend_from_slice(&0.0f32.to_le_bytes());
            p
        }));
        sec2.extend(op(0x2D, 4, &{
            let mut p = Vec::new();
            p.extend_from_slice(&0u32.to_le_bytes());
            p.extend_from_slice(b"k1a0");
            p
        }));

        let body = build(&sec2, 5, 0);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.mesh_id, *b"kir1");
        assert!(def.camera_billboard);
        assert_eq!(def.frames_per_emission, 5.0);
        assert_eq!(def.particles_per_emission, 1, "ppe 0 clamps to 1");
        assert!((def.base_position[1] - 0.2).abs() < 1e-6);
        assert_eq!(def.max_life_frames, 36.0);
        assert!(!def.is_singleton());
        assert!((def.init_scale[0] - 0.05).abs() < 1e-6);
        assert!((def.init_color[2] - 158.0 / 255.0).abs() < 1e-6);
        assert!((def.init_velocity[1] + 0.005).abs() < 1e-6);
        assert_eq!(def.alpha_track, Some(*b"k1a0"));
        assert_eq!(def.scale_x_track, None);
    }

    #[test]
    fn parses_section3_uv_scroll_and_accel() {
        // Minimal particle setup in section 2, terminated, then a section-3 stream at
        // body[0x78] with TextureCoordinateUpdater 0x27/0x28 and VelocityAccelerator 0x03.
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        let mut body = build(&setup, 1, 1);
        body.extend_from_slice(&[0u8; 4]); // terminate section 2
        let sec3_body_index = body.len();
        body[0x78..0x7C].copy_from_slice(&((sec3_body_index + 0x10) as u32).to_le_bytes());
        let mut sec3 = op(0x27, 2, &(-0.015f32).to_le_bytes());
        sec3.extend(op(0x28, 2, &0.001f32.to_le_bytes()));
        sec3.extend(op(0x03, 4, &{
            let mut p = Vec::new();
            p.extend_from_slice(&0.0f32.to_le_bytes());
            p.extend_from_slice(&(-0.02f32).to_le_bytes());
            p.extend_from_slice(&0.0f32.to_le_bytes());
            p
        }));
        body.extend_from_slice(&sec3);

        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(
            (def.uv_scroll[0] + 0.015).abs() < 1e-9,
            "0x27 -> uv_scroll[0]"
        );
        assert!(
            (def.uv_scroll[1] - 0.001).abs() < 1e-9,
            "0x28 -> uv_scroll[1]"
        );
        assert_eq!(def.accel, Some([0.0, -0.02, 0.0]), "0x03 -> accel");
    }

    // The celestial opcodes live in the section-3 updater stream (body[0x78]), NOT the
    // section-2 initializer stream, where 0x4E/0x4F mean FixedPointPositionVarianceSetup and
    // 0x45 means ParentPositionCopyConfig (research/xim ParticleGeneratorParser.kt:248-249,
    // 239 vs 444, 454-455). Reading them from the wrong stream silently yields None on every
    // real DAT, which is what left the moon on hand-tuned fallback tints.
    #[test]
    fn celestial_updaters_come_from_section3_only() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;

        let dow = |base: u8| {
            let mut p = vec![0u8; 4];
            p.extend((0..DAYS_OF_WEEK as u8).flat_map(|i| [base + i, 0, 0, 255]));
            op(0x4E, 10, &p)
        };
        let phase = || {
            let mut p = vec![0u8; 4];
            p.extend((0..MOON_PHASES as u8).flat_map(|i| [0, 0, i, 255]));
            op(0x4F, 14, &p)
        };

        // In section 2 they must be ignored outright.
        let mut sec2 = setup.clone();
        sec2.extend(dow(0));
        sec2.extend(phase());
        sec2.extend(op(0x45, 1, &[]));
        let def = ParticleGeneratorDef::parse(&build(&sec2, 1, 1))
            .unwrap()
            .unwrap();
        assert_eq!(def.day_of_week_color, None);
        assert_eq!(def.moon_phase_color, None);
        assert!(!def.moon_phase_sprite);

        // In section 3 they decode.
        let mut body = build(&setup, 1, 1);
        body.extend_from_slice(&[0u8; 4]);
        let sec3_at = body.len();
        body[0x78..0x7C].copy_from_slice(&((sec3_at + 0x10) as u32).to_le_bytes());
        let mut sec3 = op(0x45, 1, &[]);
        sec3.extend(dow(16));
        sec3.extend(phase());
        body.extend_from_slice(&sec3);

        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(def.moon_phase_sprite, "0x45 -> moon-phase sprite frame");
        let dow = def.day_of_week_color.expect("0x4E decodes");
        assert!((dow[0][0] - 16.0 / 255.0).abs() < 1e-6);
        assert!((dow[7][0] - 23.0 / 255.0).abs() < 1e-6);
        let mp = def.moon_phase_color.expect("0x4F decodes");
        assert!((mp[11][2] - 11.0 / 255.0).abs() < 1e-6);
    }

    // research/xim ParticleGeneratorParser.kt:270-274 — 0x60..0x63 are KeyFrameValueSetup
    // (track id at payload+4, same shape as the 0x27/0x28/0x2D life tracks) naming the
    // time-of-day RGBA curves; the section-3 ClockValueUpdater 0x3C..0x3F arms each channel.
    #[test]
    fn tod_color_tracks_pair_setup_with_updater() {
        let mut sec2 = op(0x01, 12, &[]);
        sec2[4 + 29] = LINKED_DATA_STATIC_MESH;
        for (opcode, id) in [(0x60u8, b"ksr1"), (0x61, b"ksg1"), (0x62, b"ksb1")] {
            let mut p = vec![0u8; 4];
            p.extend_from_slice(id);
            sec2.extend(op(opcode, 4, &p));
        }
        let mut body = build(&sec2, 1, 1);
        body.extend_from_slice(&[0u8; 4]);
        let sec3_at = body.len();
        body[0x78..0x7C].copy_from_slice(&((sec3_at + 0x10) as u32).to_le_bytes());
        // Arm red and blue only: an unarmed channel keeps its track but must not be applied.
        let mut sec3 = op(0x3C, 1, &[]);
        sec3.extend(op(0x3E, 1, &[]));
        body.extend_from_slice(&sec3);

        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(
            def.tod_color_tracks,
            [Some(*b"ksr1"), Some(*b"ksg1"), Some(*b"ksb1"), None]
        );
        assert_eq!(def.tod_color_driven, [true, false, true, false]);
    }

    // Real-DAT guard for both of the above: West Ronfaure's fine-weather celestial set is the
    // canonical shape — the sun carries three time-of-day colour curves, the moon carries a
    // phase-indexed sprite plus both tint tables. If the section split regresses these all
    // go quietly empty again.
    #[test]
    fn real_dat_west_ronfaure_celestial_generators() {
        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            eprintln!("skipping: no DAT root");
            return;
        };
        let Ok(loc) = root.resolve(201) else {
            eprintln!("skipping: file 201 unresolvable");
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            eprintln!("skipping: file 201 unreadable");
            return;
        };

        let mut saw_sun = false;
        let mut saw_moon = false;
        for c in crate::chunk::walk(&bytes).flatten() {
            if crate::kind::ChunkKind::from_u8(c.kind) != Some(crate::kind::ChunkKind::Generator) {
                continue;
            }
            let Ok(Some(def)) = ParticleGeneratorDef::parse(c.data) else {
                continue;
            };
            match (&c.name, def.attach_type) {
                (b"sun1", AttachType::Sun) => {
                    saw_sun = true;
                    assert_eq!(
                        def.tod_color_driven[..3],
                        [true, true, true],
                        "sun1 drives r/g/b from time-of-day curves"
                    );
                    assert!(def.tod_color_tracks[..3].iter().all(Option::is_some));
                }
                (b"moon", AttachType::Moon) => {
                    saw_moon = true;
                    assert!(def.moon_phase_sprite, "moon picks its frame by phase");
                    assert!(def.day_of_week_color.is_some(), "moon has a 0x4E table");
                    assert!(def.moon_phase_color.is_some(), "moon has a 0x4F table");
                }
                _ => {}
            }
        }
        assert!(saw_sun, "file 201 defines a Sun-attached `sun1`");
        assert!(saw_moon, "file 201 defines a Moon-attached `moon`");
    }

    #[test]
    fn no_section3_leaves_defaults() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        let body = build(&setup, 1, 1);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.uv_scroll, [0.0, 0.0]);
        assert_eq!(def.accel, None);
    }

    #[test]
    fn gen_flags_decode_auto_run_and_continuous() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        let mut body = build(&setup, 1, 1);
        body[0x69] = GEN_FLAG_AUTO_RUN | GEN_FLAG_CONTINUOUS;
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(def.auto_run);
        assert!(def.continuous);

        let mut body = build(&setup, 1, 1);
        body[0x69] = 0;
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(!def.auto_run);
        assert!(!def.continuous);
    }

    #[test]
    fn non_particle_setup_is_none() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = 0x47; // point light, not particle
        let body = build(&setup, 1, 1);
        assert!(ParticleGeneratorDef::parse(&body).unwrap().is_none());

        // 0x57 Null particle type is likewise non-visual and rejected.
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = 0x57;
        let body = build(&setup, 1, 1);
        assert!(ParticleGeneratorDef::parse(&body).unwrap().is_none());
    }

    // Regression pin (Poison's venom cloud): a 0x0E SpriteSheet generator used to be dropped
    // because only 0x0B was accepted. It must now parse to Some with mesh_kind == SpriteSheet.
    #[test]
    fn sprite_sheet_setup_parses_with_mesh_kind() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 8..4 + 12].copy_from_slice(b"fir ");
        setup[4 + 29] = LINKED_DATA_SPRITE_SHEET;
        setup[4 + 30..4 + 32].copy_from_slice(&24u16.to_le_bytes());
        let body = build(&setup, 1, 1);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.mesh_kind, ParticleMeshKind::SpriteSheet);
        assert_eq!(def.mesh_id, *b"fir ");
        assert_eq!(def.max_life_frames, 24.0);
    }

    // research/xim ParticleInitializers.kt:105-116 — renderStateFlags is the u16 after the
    // billboard flags; 0x1000 picks retail's NonZeroOneTSS element (CMoD3m.cpp:363-370).
    #[test]
    fn render_state_flag_selects_ignore_texture_alpha_element() {
        let element = |render_state: u16| {
            let mut setup = op(0x01, 12, &[]);
            setup[4 + 2..4 + 4].copy_from_slice(&render_state.to_le_bytes());
            setup[4 + 29] = LINKED_DATA_STATIC_MESH;
            let body = build(&setup, 1, 1);
            ParticleGeneratorDef::parse(&body)
                .unwrap()
                .unwrap()
                .ignore_texture_alpha
        };
        assert!(!element(0x0000));
        assert!(!element(0x0FFF), "only bit 0x1000 selects the element");
        assert!(element(0x1000));
        assert!(element(0x1200), "other render-state bits do not mask it");
    }

    // CMoD3m.cpp:345-349 keys the TEXTUREFACTOR-alpha promotion on the exact blend byte, which
    // the ParticleBlend collapse (0x03/0x44/0x64 all -> Blend) cannot express.
    #[test]
    fn blend_byte_survives_the_blend_func_collapse() {
        let parsed = |p0: u8| {
            let mut setup = op(0x01, 12, &[]);
            setup[4 + 29] = LINKED_DATA_STATIC_MESH;
            setup.extend(op(0x1E, 2, &[p0, 0, 0, 0]));
            let body = build(&setup, 1, 1);
            let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
            (def.blend, def.blend_byte)
        };
        assert_eq!(parsed(0x44), (ParticleBlend::Blend, 0x44));
        assert_eq!(parsed(0x64), (ParticleBlend::Blend, 0x64));
        assert_eq!(parsed(0x48), (ParticleBlend::Additive, 0x48));
    }

    #[test]
    fn static_mesh_setup_reports_static_mesh_kind() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        let body = build(&setup, 1, 1);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.mesh_kind, ParticleMeshKind::StaticMesh);
    }

    #[test]
    fn singleton_when_max_life_zero() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 8..4 + 12].copy_from_slice(b"sea0");
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;
        // max life left at 0
        let body = build(&setup, 1, 1);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(def.is_singleton());
    }

    // Pins the XIM attachFlags bit layout (ParticleGeneratorParser.kt:23-33) against the
    // ground-truth word 0x5402 read out of Poison's effect DAT (file 3020).
    #[test]
    fn attach_flags_split_type_and_joints() {
        let mut setup = op(0x01, 12, &[]);
        setup[4 + 29] = LINKED_DATA_STATIC_MESH;

        let body = build_attached(&setup, 1, 1, 0x5402, ATTACH_SOURCE_ORIENTED);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.attach_type, AttachType::TargetActor);
        assert_eq!(def.attach_joint_source, 0);
        assert_eq!(def.attach_joint_target, 21);
        assert!(def.attach_source_oriented);

        let body = build_attached(&setup, 1, 1, 0x5402, 0);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert!(!def.attach_source_oriented);

        // Joint 0 lives in bits 4..10, joint 1 in bits 10..16, type in the low nibble.
        let body = build_attached(&setup, 1, 1, 0x0409 | (7 << ATTACH_JOINT0_SHIFT), 0);
        let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
        assert_eq!(def.attach_type, AttachType::SourceActorWeapon);
        assert_eq!(def.attach_joint_source, 7);
        assert_eq!(def.attach_joint_target, 1);

        // 0x7 / 0x8 / 0xD are not AttachType flags; XIM warns and falls back to None.
        for unknown in [0x7u16, 0x8, 0xD] {
            assert_eq!(AttachType::from_flag(unknown), None);
            let body = build_attached(&setup, 1, 1, unknown, 0);
            let def = ParticleGeneratorDef::parse(&body).unwrap().unwrap();
            assert_eq!(def.attach_type, AttachType::None);
        }
    }

    // Real-DAT guard: every generator in Poison's completion-effect file attaches to the
    // target actor at joint 21, which is what makes the venom cloud land on the victim.
    #[test]
    fn real_dat_poison_generators_attach_to_target() {
        const POISON_EFFECT_FILE_ID: u32 = 3020;
        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            return;
        };
        let Ok(loc) = root.resolve(POISON_EFFECT_FILE_ID) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            return;
        };
        let mut seen = 0;
        for c in crate::chunk::walk(&bytes).flatten() {
            if crate::kind::ChunkKind::from_u8(c.kind) != Some(crate::kind::ChunkKind::Generator) {
                continue;
            }
            let Ok(Some(def)) = ParticleGeneratorDef::parse(c.data) else {
                continue;
            };
            seen += 1;
            assert_eq!(
                def.attach_type,
                AttachType::TargetActor,
                "generator {}",
                String::from_utf8_lossy(&c.name)
            );
            assert_eq!(def.attach_joint_target, 21);
        }
        assert!(seen > 0, "no particle generators parsed from file 3020");
    }

    #[test]
    fn keyframe_track_interpolates_and_clamps() {
        let mut b = Vec::new();
        for (t, v) in [(0.0f32, 0.0f32), (0.5, 0.22), (1.0, 0.12)] {
            b.extend_from_slice(&t.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        let kf = KeyFrameTrack::parse(&b);
        assert_eq!(kf.points.len(), 3);
        assert!((kf.sample(0.0) - 0.0).abs() < 1e-6);
        assert!((kf.sample(0.25) - 0.11).abs() < 1e-6);
        assert!((kf.sample(0.5) - 0.22).abs() < 1e-6);
        assert!((kf.sample(0.75) - 0.17).abs() < 1e-6);
        assert!((kf.sample(1.5) - 0.12).abs() < 1e-6, "clamps to last");
    }

    #[test]
    fn keyframe_stops_at_time_one() {
        let mut b = Vec::new();
        b.extend_from_slice(&0.0f32.to_le_bytes());
        b.extend_from_slice(&0.0f32.to_le_bytes());
        b.extend_from_slice(&1.0f32.to_le_bytes());
        b.extend_from_slice(&0.5f32.to_le_bytes());
        // trailing garbage past the terminator must be ignored
        b.extend_from_slice(&[0xAA; 16]);
        let kf = KeyFrameTrack::parse(&b);
        assert_eq!(kf.points.len(), 2);
    }
}
