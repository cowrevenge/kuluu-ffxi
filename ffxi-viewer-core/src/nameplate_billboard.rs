use std::sync::Arc;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ffxi_viewer_wire::EntityKind;

use crate::camera::{nameplate_anchor_y, CameraMode, OperatorCamera};
use crate::components::{InGameEntity, Nameplate, WorldEntity};
use crate::scene::{BakedActor, Target};
// Retail advances the targeted-nameplate pulse once per rendered frame.
use crate::scheduler_runtime::RETAIL_FPS;
use crate::snapshot::SceneState;

const NAME_PX: f32 = 64.0;

// research/XIClient/src/XIClient/source/Game/GameManager.cpp:798-799 — retail's clip planes
// are fixed, so the nameplate ramp below must not read our camera's user-tunable projection.
const RETAIL_NEAR_CLIP_YALMS: f32 = 0.1;
const RETAIL_FAR_CLIP_YALMS: f32 = 65535.0;

// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:75
const NDC_DEPTH_FIXED_POINT_SCALE: u32 = 4096;
// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:261-262 rejects
// z >= 1.0, so the deepest drawable fixed-point depth is one step short of the scale.
const MAX_DRAWABLE_DEPTH_FIXED: u32 = NDC_DEPTH_FIXED_POINT_SCALE - 1;
// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:90-91
const FADE_START_DEPTH_FIXED: u32 = 0xFB4;
const FADE_END_DEPTH_FIXED: u32 = 0x1004;
// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:72-73 — the
// reciprocal-w gate (1/depth < 1) drops names inside one yalm of the view plane.
const MIN_VIEW_DEPTH_YALMS: f32 = 1.0;

// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:31 — glyph units
// to viewport fraction, applied to a pre-transformed (RHW=1) screen-space quad.
const NAME_SCREEN_SCALE: f32 = 0.002_343_75;
// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:35 — one name
// line is one glyph cell tall.
const NAME_LINE_HEIGHT_UNITS: f32 = 8.0;
const NAME_LINE_SCREEN_FRACTION: f32 = NAME_SCREEN_SCALE * NAME_LINE_HEIGHT_UNITS;

// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:111-112
const TARGET_PULSE_DEGREES_PER_FRAME: u32 = 16;
const FULL_TURN_DEGREES: u32 = 360;
const TARGET_PULSE_AMPLITUDE: f32 = 32.0;
const TARGET_PULSE_BIAS: f32 = 96.0;
// research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp:115 repacks
// the product as `(scaledAlpha & 0xFFFFFF80) << 17`, i.e. a shift right by 7.
const TARGET_PULSE_DIVISOR: f32 = 128.0;

const OUTLINE_RADIUS_PX: i32 = 3;

const OUTLINE_COLOR: [u8; 4] = [0, 0, 0, 220];

const HP_BAR_HEIGHT_PX: u32 = 16;

const HP_BAR_TOP_GAP_PX: u32 = 8;

const HP_BAR_WIDTH_FRACTION: f32 = 1.0;

#[derive(Resource)]
pub struct BillboardFont(pub Arc<FontArc>);

impl FromWorld for BillboardFont {
    fn from_world(_: &mut World) -> Self {
        let font = FontArc::try_from_slice(crate::ui_font::DEJAVU_SANS_MONO)
            .expect("bundled DejaVuSansMono.ttf must parse as a valid TTF for ab_glyph");
        Self(Arc::new(font))
    }
}

#[derive(Component)]
pub struct NameplateBillboard {
    pub entity_id: u32,
    pub kind: EntityKind,

    pub base_name: String,

    pub last_rendered: String,

    pub last_color: [u8; 4],

    pub last_hp: Option<u8>,

    pub last_alpha: f32,
}

#[derive(Component)]
pub struct BillboardAspect {
    pub width: u32,
    pub height: u32,
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_nameplate_billboard(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    font: &FontArc,
    entity_id: u32,
    kind: EntityKind,
    name: &str,
    color: Color,
) -> Entity {
    let rgba = color_to_rgba8(color);

    let raster = rasterize_text_to_image(font, name, NAME_PX, rgba, None).clone();
    let aspect = (raster.width(), raster.height());
    let image_handle = images.add(raster);

    let mesh_handle = meshes.add(Rectangle::new(1.0, 1.0));

    let material_handle = materials.add(StandardMaterial {
        base_color_texture: Some(image_handle),
        base_color: Color::WHITE,

        unlit: true,
        alpha_mode: AlphaMode::Blend,

        cull_mode: None,
        ..default()
    });

    commands
        .spawn((
            InGameEntity,
            Nameplate { entity_id, kind },
            NameplateBillboard {
                entity_id,
                kind,
                base_name: name.to_string(),
                last_rendered: name.to_string(),
                last_color: rgba,
                last_hp: None,
                last_alpha: 1.0,
            },
            BillboardAspect {
                width: aspect.0,
                height: aspect.1,
            },
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
            Transform::from_translation(Vec3::new(0.0, -1_000_000.0, 0.0)),
            Visibility::Hidden,
            NotShadowCaster,
            NotShadowReceiver,
        ))
        .id()
}

pub fn is_self_billboard(entity_id: u32, self_char_id: Option<u32>) -> bool {
    self_char_id.is_some_and(|cid| cid != 0 && cid == entity_id)
}

/// Retail draws the self plate in the same PC styling as other players
/// (kuluu-hof), but in first-person the plate anchors just above the camera
/// eye — a near-degenerate projection that dips/jitters on stutter frames
/// (kuluu-gr2) and occludes the view — so it is hidden there.
pub fn self_plate_hidden(is_self: bool, mode: CameraMode) -> bool {
    is_self && matches!(mode, CameraMode::FirstPerson)
}

pub fn nameplate_color(kind: EntityKind, engaged: bool, dead: bool) -> Color {
    match kind {
        EntityKind::Pc => Color::srgb(0.55, 0.95, 1.0),
        EntityKind::Npc => Color::srgb(0.55, 1.0, 0.55),
        EntityKind::Mob => {
            if dead {
                Color::srgb(0.55, 0.55, 0.55)
            } else if engaged {
                Color::srgb(1.0, 0.55, 0.25)
            } else {
                Color::srgb(1.0, 0.95, 0.7)
            }
        }
        EntityKind::Pet => Color::srgb(0.55, 0.95, 0.65),
        EntityKind::Other => Color::srgb(0.85, 0.85, 0.85),
    }
}

pub fn format_billboard_label(base_name: &str, _hp_pct: Option<u8>, _kind: EntityKind) -> String {
    base_name.to_string()
}

pub fn update_nameplate_billboards_system(
    state: Res<SceneState>,
    camera_mode: Res<CameraMode>,
    time: Res<Time>,
    target: Res<Target>,
    cam_q: Query<(&Transform, &Projection), (With<OperatorCamera>, Without<NameplateBillboard>)>,
    world_q: Query<(&Transform, &WorldEntity, Option<&BakedActor>), Without<NameplateBillboard>>,
    mut billboards: Query<(
        Entity,
        &mut NameplateBillboard,
        &mut BillboardAspect,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    font: Res<BillboardFont>,
    mut commands: Commands,
) {
    let Ok((cam_t, projection)) = cam_q.single() else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let cam_pos = cam_t.translation;
    let cam_forward = Vec3::from(cam_t.forward());
    let half_fov_tan = (perspective.fov * 0.5).tan();
    let line_px = text_line_height_px(&font.0, NAME_PX) as f32;
    let pulse_frame = (time.elapsed_secs() * RETAIL_FPS) as u32;

    let mut pos_by_id: std::collections::HashMap<u32, (Vec3, f32)> =
        std::collections::HashMap::with_capacity(world_q.iter().len());
    for (t, w, baked) in &world_q {
        pos_by_id.insert(w.id, (t.translation, nameplate_anchor_y(baked)));
    }

    let self_char_id: Option<u32> = state.snapshot.self_char_id;
    let mut hp_by_id: std::collections::HashMap<u32, Option<u8>> = std::collections::HashMap::new();
    let mut claim_by_id: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for ent in &state.snapshot.entities {
        hp_by_id.insert(ent.id, ent.hp_pct);
        claim_by_id.insert(ent.id, ent.claim_id);
    }

    for (ui_entity, mut np, mut aspect, mut transform, mut vis, mat) in &mut billboards {
        if self_plate_hidden(is_self_billboard(np.entity_id, self_char_id), *camera_mode) {
            *vis = Visibility::Hidden;
            continue;
        }

        let Some(&(entity_pos, head_y_offset)) = pos_by_id.get(&np.entity_id) else {
            commands.entity(ui_entity).try_despawn();
            continue;
        };

        let head_pos = entity_pos + Vec3::Y * head_y_offset;
        let view_depth = (head_pos - cam_pos).dot(cam_forward);
        let Some(scale) = scale_for_view_depth(view_depth) else {
            *vis = Visibility::Hidden;
            continue;
        };

        let aspect_ratio = aspect.width.max(1) as f32 / aspect.height.max(1) as f32;
        let plate_to_line = aspect.height.max(1) as f32 / line_px;
        let viewport_height_yalms = 2.0 * view_depth * half_fov_tan;
        let world_height =
            viewport_height_yalms * NAME_LINE_SCREEN_FRACTION * plate_to_line * scale;
        let world_width = world_height * aspect_ratio;

        transform.translation = head_pos;
        transform.rotation = cam_t.rotation;
        transform.scale = Vec3::new(world_width, world_height, 1.0);
        *vis = Visibility::Visible;

        let want_alpha = if target.id == Some(np.entity_id) {
            target_alpha_pulse(pulse_frame)
        } else {
            1.0
        };
        if want_alpha != np.last_alpha {
            if let Some(mut mat_data) = materials.get_mut(&mat.0) {
                mat_data.base_color = Color::WHITE.with_alpha(want_alpha);
                np.last_alpha = want_alpha;
            }
        }

        let engaged = matches!(np.kind, EntityKind::Mob)
            && self_char_id.is_some_and(|cid| {
                cid != 0 && claim_by_id.get(&np.entity_id).copied() == Some(cid)
            });

        let dead = matches!(np.kind, EntityKind::Mob)
            && hp_by_id.get(&np.entity_id).copied().flatten() == Some(0);
        let want_color = color_to_rgba8(nameplate_color(np.kind, engaged, dead));

        let snapshot_hp = hp_by_id.get(&np.entity_id).copied().flatten();
        let want_hp = if matches!(np.kind, EntityKind::Mob | EntityKind::Pet) {
            snapshot_hp
        } else {
            None
        };

        let want = format_billboard_label(&np.base_name, snapshot_hp, np.kind);
        if want != np.last_rendered || want_color != np.last_color || want_hp != np.last_hp {
            if let Some(mat_data) = materials.get_mut(&mat.0) {
                if let Some(handle) = mat_data.base_color_texture.clone() {
                    crate::perf_probe::note_nameplate_raster();
                    let new_img =
                        rasterize_text_to_image(&font.0, &want, NAME_PX, want_color, want_hp);
                    aspect.width = new_img.width();
                    aspect.height = new_img.height();
                    let _ = images.insert(&handle, new_img);
                    np.last_rendered = want;
                    np.last_color = want_color;
                    np.last_hp = want_hp;
                }
            }
        }
    }
}

pub fn view_depth_to_fixed_point(view_depth_yalms: f32) -> Option<u32> {
    if view_depth_yalms <= MIN_VIEW_DEPTH_YALMS {
        return None;
    }
    let z_ndc = RETAIL_FAR_CLIP_YALMS / (RETAIL_FAR_CLIP_YALMS - RETAIL_NEAR_CLIP_YALMS)
        * (1.0 - RETAIL_NEAR_CLIP_YALMS / view_depth_yalms);
    if z_ndc < 0.0 {
        return None;
    }
    let depth_fixed = (z_ndc * NDC_DEPTH_FIXED_POINT_SCALE as f32) as u32;
    (depth_fixed <= MAX_DRAWABLE_DEPTH_FIXED).then_some(depth_fixed)
}

pub fn scale_for_view_depth(view_depth_yalms: f32) -> Option<f32> {
    let depth_fixed = view_depth_to_fixed_point(view_depth_yalms)?;
    if depth_fixed > FADE_END_DEPTH_FIXED {
        return None;
    }
    if depth_fixed < FADE_START_DEPTH_FIXED {
        return Some(1.0);
    }
    Some(
        (FADE_END_DEPTH_FIXED - depth_fixed) as f32
            / (FADE_END_DEPTH_FIXED - FADE_START_DEPTH_FIXED) as f32,
    )
}

pub fn target_alpha_pulse(frame: u32) -> f32 {
    let angle_deg = frame.wrapping_mul(TARGET_PULSE_DEGREES_PER_FRAME) % FULL_TURN_DEGREES;
    ((angle_deg as f32).to_radians().sin() * TARGET_PULSE_AMPLITUDE + TARGET_PULSE_BIAS)
        / TARGET_PULSE_DIVISOR
}

fn text_line_height_px(font: &FontArc, px: f32) -> u32 {
    let scaled = font.as_scaled(PxScale::from(px));
    (scaled.ascent() - scaled.descent()).ceil().max(1.0) as u32
}

fn rasterize_text_to_image(
    font: &FontArc,
    text: &str,
    px: f32,
    color: [u8; 4],
    hp_pct: Option<u8>,
) -> Image {
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let ascent = scaled.ascent();
    let line_h = text_line_height_px(font, px);

    let mut pen_x = 0.0_f32;
    let mut max_x = 0.0_f32;
    let mut glyphs = Vec::with_capacity(text.chars().count());
    let mut prev = None;
    for ch in text.chars() {
        let g = scaled.scaled_glyph(ch);
        if let Some(p) = prev {
            pen_x += scaled.kern(p, g.id);
        }
        let advance = scaled.h_advance(g.id);

        let positioned = ab_glyph::Glyph {
            id: g.id,
            position: ab_glyph::point(pen_x, ascent),
            scale: g.scale,
        };
        pen_x += advance;
        max_x = max_x.max(pen_x);
        prev = Some(positioned.id);
        glyphs.push(positioned);
    }

    let pad = (OUTLINE_RADIUS_PX + 1) as u32;
    let width = (max_x.ceil() as u32).max(1) + 2 * pad;
    let text_height = line_h + 2 * pad;

    let hp_strip = HP_BAR_TOP_GAP_PX + HP_BAR_HEIGHT_PX;
    let height = text_height + hp_strip;

    let mut coverage = vec![0u8; (width * height) as usize];
    for glyph in glyphs {
        if let Some(outline_glyph) = scaled.outline_glyph(glyph) {
            let bb = outline_glyph.px_bounds();
            outline_glyph.draw(|gx, gy, c| {
                let px_x = bb.min.x as i32 + gx as i32 + pad as i32;
                let px_y = bb.min.y as i32 + gy as i32 + pad as i32;
                if px_x < 0 || px_y < 0 || px_x >= width as i32 || px_y >= text_height as i32 {
                    return;
                }
                let i = (px_y as u32 * width + px_x as u32) as usize;
                let added = (c * 255.0).round().clamp(0.0, 255.0) as u8;
                coverage[i] = coverage[i].saturating_add(added);
            });
        }
    }

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let r = OUTLINE_RADIUS_PX;
    let r2 = r * r;
    let w_i = width as i32;
    let text_h_i = text_height as i32;
    for y in 0..text_h_i {
        for x in 0..w_i {
            let text_alpha = coverage[(y * w_i + x) as usize];

            let mut outline_alpha: u8 = 0;
            let y0 = (y - r).max(0);
            let y1 = (y + r).min(text_h_i - 1);
            let x0 = (x - r).max(0);
            let x1 = (x + r).min(w_i - 1);
            for ny in y0..=y1 {
                let dy = ny - y;
                let dy2 = dy * dy;
                for nx in x0..=x1 {
                    let dx = nx - x;
                    if dx * dx + dy2 > r2 {
                        continue;
                    }
                    let na = coverage[(ny * w_i + nx) as usize];
                    if na > outline_alpha {
                        outline_alpha = na;
                    }
                }
            }

            let ta = (text_alpha as f32 / 255.0) * (color[3] as f32 / 255.0);
            let oa = (outline_alpha as f32 / 255.0) * (OUTLINE_COLOR[3] as f32 / 255.0);
            let out_a = ta + (1.0 - ta) * oa;
            if out_a <= 0.0 {
                continue;
            }
            let inv = 1.0 / out_a;
            let or = color[0] as f32 * ta + OUTLINE_COLOR[0] as f32 * (1.0 - ta) * oa;
            let og = color[1] as f32 * ta + OUTLINE_COLOR[1] as f32 * (1.0 - ta) * oa;
            let ob = color[2] as f32 * ta + OUTLINE_COLOR[2] as f32 * (1.0 - ta) * oa;
            let pi = ((y * w_i + x) * 4) as usize;
            pixels[pi] = (or * inv).round().clamp(0.0, 255.0) as u8;
            pixels[pi + 1] = (og * inv).round().clamp(0.0, 255.0) as u8;
            pixels[pi + 2] = (ob * inv).round().clamp(0.0, 255.0) as u8;
            pixels[pi + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    if let Some(pct) = hp_pct {
        let bar_pixel_w = (width as f32 * HP_BAR_WIDTH_FRACTION) as u32;
        let bar_x = (width.saturating_sub(bar_pixel_w)) / 2;
        let bar_y = text_height + HP_BAR_TOP_GAP_PX;
        let bar_h = HP_BAR_HEIGHT_PX;
        let fill_color = hp_color_rgba(pct);

        for x in 0..bar_pixel_w {
            paint_pixel(&mut pixels, width, bar_x + x, bar_y, OUTLINE_COLOR);
            paint_pixel(
                &mut pixels,
                width,
                bar_x + x,
                bar_y + bar_h - 1,
                OUTLINE_COLOR,
            );
        }
        for y in 0..bar_h {
            paint_pixel(&mut pixels, width, bar_x, bar_y + y, OUTLINE_COLOR);
            paint_pixel(
                &mut pixels,
                width,
                bar_x + bar_pixel_w - 1,
                bar_y + y,
                OUTLINE_COLOR,
            );
        }

        let interior_w = bar_pixel_w.saturating_sub(2);
        let fill_w = (interior_w as f32 * pct.min(100) as f32 / 100.0).round() as u32;
        for y in 1..(bar_h - 1) {
            for x in 0..fill_w {
                paint_pixel(&mut pixels, width, bar_x + 1 + x, bar_y + y, fill_color);
            }
        }
    }

    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

fn color_to_rgba8(c: Color) -> [u8; 4] {
    let s = c.to_srgba();
    [
        (s.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (s.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (s.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (s.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

#[inline]
fn paint_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let pi = ((y * width + x) * 4) as usize;
    if pi + 4 > pixels.len() {
        return;
    }
    pixels[pi] = color[0];
    pixels[pi + 1] = color[1];
    pixels[pi + 2] = color[2];
    pixels[pi + 3] = color[3];
}

fn hp_color_rgba(pct: u8) -> [u8; 4] {
    let f = (pct.min(100) as f32) / 100.0;
    let (r, g) = if f >= 0.5 {
        let t = (1.0 - f) * 2.0;
        (t, 1.0)
    } else {
        let t = f * 2.0;
        (1.0, t)
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, 0, 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_billboard_matches_known_self_id() {
        assert!(is_self_billboard(0xCAFE, Some(0xCAFE)));
    }

    #[test]
    fn other_entities_are_not_self() {
        assert!(!is_self_billboard(0x4242, Some(0xCAFE)));
    }

    #[test]
    fn unknown_self_id_matches_nothing() {
        assert!(!is_self_billboard(0xCAFE, None));
    }

    #[test]
    fn zero_self_id_is_unresolved_not_a_match() {
        assert!(!is_self_billboard(0, Some(0)));
    }

    #[test]
    fn self_plate_hidden_only_in_first_person() {
        assert!(self_plate_hidden(true, CameraMode::FirstPerson));
        assert!(!self_plate_hidden(true, CameraMode::Chase));
    }

    #[test]
    fn other_plates_visible_in_both_camera_modes() {
        assert!(!self_plate_hidden(false, CameraMode::FirstPerson));
        assert!(!self_plate_hidden(false, CameraMode::Chase));
    }

    const SCALE_EPSILON: f32 = 1e-5;
    // The deepest drawable plate: (0x1004 - 4095) / 80.
    const SCALE_FLOOR: f32 = 0.0625;

    #[test]
    fn scale_ramp_matches_retail_depth_table() {
        let table = [
            (3.0_f32, 1.0_f32),
            (5.0, 1.0),
            (5.38, 1.0),
            (5.5, 0.9875),
            (10.0, 0.5625),
            (20.0, 0.3125),
            (50.0, 0.1625),
            (100.0, 0.1125),
            (500.0, SCALE_FLOOR),
            (5000.0, SCALE_FLOOR),
        ];
        for (depth, want) in table {
            let got = scale_for_view_depth(depth).expect("depth is inside the drawable range");
            assert!(
                (got - want).abs() < SCALE_EPSILON,
                "depth {depth}: got {got}, want {want}"
            );
        }
    }

    #[test]
    fn plateau_ends_at_the_fade_start_depth() {
        assert_eq!(
            view_depth_to_fixed_point(5.38),
            Some(FADE_START_DEPTH_FIXED - 1)
        );
        assert_eq!(view_depth_to_fixed_point(5.4), Some(FADE_START_DEPTH_FIXED));
        assert_eq!(scale_for_view_depth(5.38), Some(1.0));
        assert_eq!(scale_for_view_depth(5.4), Some(1.0));
        assert!(scale_for_view_depth(5.5).unwrap() < 1.0);
    }

    #[test]
    fn deepest_drawable_plate_sits_on_the_floor() {
        assert_eq!(
            view_depth_to_fixed_point(5000.0),
            Some(MAX_DRAWABLE_DEPTH_FIXED)
        );
        let floor = (FADE_END_DEPTH_FIXED - MAX_DRAWABLE_DEPTH_FIXED) as f32
            / (FADE_END_DEPTH_FIXED - FADE_START_DEPTH_FIXED) as f32;
        assert!((floor - SCALE_FLOOR).abs() < SCALE_EPSILON);
    }

    #[test]
    fn plates_inside_one_yalm_of_the_view_plane_are_dropped() {
        assert_eq!(scale_for_view_depth(1.0), None);
        assert_eq!(scale_for_view_depth(0.5), None);
        assert_eq!(scale_for_view_depth(-10.0), None);
    }

    #[test]
    fn plates_past_the_far_clip_are_dropped() {
        assert_eq!(scale_for_view_depth(1.0e6), None);
    }

    #[test]
    fn scale_never_grows_with_depth() {
        let mut prev = 1.0_f32;
        for step in 2..2000 {
            let depth = step as f32 * 0.5;
            let Some(scale) = scale_for_view_depth(depth) else {
                continue;
            };
            assert!(scale <= prev + SCALE_EPSILON, "depth {depth} scaled up");
            assert!(
                scale >= SCALE_FLOOR - SCALE_EPSILON,
                "depth {depth} below floor"
            );
            prev = scale;
        }
    }

    #[test]
    fn target_alpha_pulse_breathes_between_half_and_full() {
        let trough = (TARGET_PULSE_BIAS - TARGET_PULSE_AMPLITUDE) / TARGET_PULSE_DIVISOR;
        let crest = (TARGET_PULSE_BIAS + TARGET_PULSE_AMPLITUDE) / TARGET_PULSE_DIVISOR;
        for frame in 0..FULL_TURN_DEGREES {
            let alpha = target_alpha_pulse(frame);
            assert!((trough..=crest).contains(&alpha), "frame {frame}: {alpha}");
        }
        assert!(
            (target_alpha_pulse(0) - TARGET_PULSE_BIAS / TARGET_PULSE_DIVISOR).abs()
                < SCALE_EPSILON
        );
    }

    #[test]
    fn target_alpha_pulse_period_is_a_full_turn_of_frames() {
        fn gcd(a: u32, b: u32) -> u32 {
            if b == 0 {
                a
            } else {
                gcd(b, a % b)
            }
        }
        let period = FULL_TURN_DEGREES / gcd(FULL_TURN_DEGREES, TARGET_PULSE_DEGREES_PER_FRAME);
        for frame in 0..period {
            assert_eq!(
                target_alpha_pulse(frame),
                target_alpha_pulse(frame + period)
            );
        }
    }
}
