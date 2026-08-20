use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ffxi_dat::main_dll::{MainDll, ZoneMapRecord};
use ffxi_dat::map_image::{parse_graphic, scan_graphics, GraphicFlag};

use crate::snapshot::SceneState;

use super::{MinimapAabb, MinimapState, RetailStatus};

#[derive(Resource, Default)]
pub struct MapCalibration {
    dll: Option<std::sync::Arc<MainDll>>,
    tried: bool,
}

impl MapCalibration {
    pub(crate) fn ensure_dll(&mut self, root: &std::path::Path) -> Option<std::sync::Arc<MainDll>> {
        if !self.tried {
            self.tried = true;
            self.dll = MainDll::load(root).ok().map(std::sync::Arc::new);
        }
        self.dll.clone()
    }
}

#[derive(Resource, Default)]
pub struct PlayerMapGrid {
    pub aabb: Option<MinimapAabb>,
    zone: Option<u16>,
}

const MENUMAP_TEX: f32 = 512.0;

pub(crate) fn zone_map_to_aabb(rec: &ZoneMapRecord) -> MinimapAabb {
    let size = rec.size as f32;
    let off_x = rec.x_offset as f32;
    let off_y = rec.y_offset as f32;
    let min_x = -size * (0.5 - off_x) / MENUMAP_TEX;
    let min_y = size * (0.5 + off_y) / MENUMAP_TEX;
    MinimapAabb {
        min: Vec2::new(min_x, min_y),
        max: Vec2::new(min_x + size, min_y + size),
    }
}

/// Decode the map image one [`ZoneMapRecord`] names, calibrated from that same
/// record. Taking both from one row is the point: the record's `file_id` and its
/// offsets describe the same picture, whereas the POLUtils map table is keyed on
/// a dense ordinal that does not line up with `sub_zone_id` — a quarter of the
/// zones number their maps from 1 (kuluu-bqm5).
pub fn load_zone_map_image(
    dat_root: &ffxi_dat::DatRoot,
    rec: &ZoneMapRecord,
    images: &mut Assets<Image>,
) -> Option<(Handle<Image>, MinimapAabb)> {
    let path = dat_root.resolve(rec.file_id).ok()?.path_under(dat_root);
    let bytes = std::fs::read(&path).ok()?;
    let graphic = scan_graphics(&bytes).max_by_key(|g| g.width * g.height)?;
    let mut image = Image::new(
        Extent3d {
            width: graphic.width,
            height: graphic.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        graphic.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Some((images.add(image), zone_map_to_aabb(rec)))
}

/// The map a zone opens on: the first whose calibrated box holds the player,
/// else the zone's first. Retail instead reads the map id off the collision
/// volume the player stands in (research/xim `MapDrawer.getCollision`), which
/// this crate does not surface; picking by containment agrees with it wherever
/// the boxes are disjoint.
pub fn zone_map_for_player(
    dll: &MainDll,
    zone: u16,
    player_xz: Option<Vec2>,
) -> Option<ZoneMapRecord> {
    let records = dll.zone_maps(zone);
    player_xz
        .and_then(|p| {
            records.iter().find(|rec| {
                let aabb = zone_map_to_aabb(rec);
                (aabb.min.x..=aabb.max.x).contains(&p.x) && (aabb.min.y..=aabb.max.y).contains(&p.y)
            })
        })
        .or_else(|| records.first())
        .copied()
}

#[derive(Resource, Default, Clone)]
pub struct MinimapDatRoot(pub Option<std::sync::Arc<ffxi_dat::DatRoot>>);

/// The chosen map, carrying its own file id and calibration so the decode below
/// cannot pair the image with a different zone's box.
#[derive(Message, Debug, Clone, Copy)]
pub struct LoadRetailMapRequest {
    pub record: ZoneMapRecord,
}

pub struct RetailBackendPlugin;

impl Plugin for RetailBackendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapDatRoot>()
            .init_resource::<MapCalibration>()
            .init_resource::<PlayerMapGrid>()
            .add_message::<LoadRetailMapRequest>()
            .add_systems(
                Update,
                (
                    auto_load_retail_for_zone_system,
                    process_load_retail_map_requests,
                )
                    .chain(),
            )
            .add_systems(Update, update_player_map_grid);
    }
}

pub fn update_player_map_grid(
    scene_state: Res<SceneState>,
    dat_root: Res<MinimapDatRoot>,
    mut calib: ResMut<MapCalibration>,
    mut grid: ResMut<PlayerMapGrid>,
) {
    let zone = scene_state.snapshot.zone_id;
    if grid.zone == zone {
        return;
    }
    grid.zone = zone;
    grid.aabb = None;

    let Some(zone_id) = zone.filter(|&z| z != 0) else {
        return;
    };
    let Some(root) = dat_root.0.as_ref() else {
        return;
    };
    let Some(dll) = calib.ensure_dll(root.root()) else {
        return;
    };
    grid.aabb = zone_map_for_player(&dll, zone_id, Some(player_map_xz(&scene_state)))
        .map(|rec| zone_map_to_aabb(&rec));
}

/// The player's position on the map plane: the world XZ the marker overlay maps
/// against, so a containment test here means the same thing it does there.
fn player_map_xz(scene_state: &SceneState) -> Vec2 {
    crate::scene::ffxi_to_bevy(scene_state.snapshot.self_pos.pos).xz()
}

fn graphic_flags_present(bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    let mut i = 0usize;
    while i + 61 <= bytes.len() {
        let bmi = u32::from_le_bytes([bytes[i + 17], bytes[i + 18], bytes[i + 19], bytes[i + 20]]);
        if bmi == 40 {
            if let Some(gf) = GraphicFlag::from_u8(bytes[i]) {
                let width = i32::from_le_bytes([
                    bytes[i + 21],
                    bytes[i + 22],
                    bytes[i + 23],
                    bytes[i + 24],
                ]);
                let height = i32::from_le_bytes([
                    bytes[i + 25],
                    bytes[i + 26],
                    bytes[i + 27],
                    bytes[i + 28],
                ]);
                let bit_count = u16::from_le_bytes([bytes[i + 31], bytes[i + 32]]);
                let compression = u32::from_le_bytes([
                    bytes[i + 33],
                    bytes[i + 34],
                    bytes[i + 35],
                    bytes[i + 36],
                ]);
                let why = match parse_graphic(&bytes[i..]) {
                    Ok(Some(_)) => "ok".to_string(),
                    Ok(None) => "skipped".to_string(),
                    Err(e) => e.to_string(),
                };
                found.push(format!(
                    "{gf:?}(w={width} h={height} bpp={bit_count} compr={compression}): {why}"
                ));
                if found.len() >= 3 {
                    break;
                }
            }
        }
        i += 1;
    }
    found
}

pub fn auto_load_retail_for_zone_system(
    scene_state: Res<SceneState>,
    dat_root: Res<MinimapDatRoot>,
    mut calib: ResMut<MapCalibration>,
    mut state: ResMut<MinimapState>,
    mut writer: MessageWriter<LoadRetailMapRequest>,
) {
    let Some(zone_id) = scene_state.snapshot.zone_id else {
        return;
    };
    if zone_id == 0 {
        return;
    }

    // The zone-map table is keyed by zone id, which inside the Mog House still
    // names the surrounding city — there is no retail map for the interior, so
    // drop retail mode and let the TopDown cull-bake re-bake from the MH geometry.
    if scene_state.snapshot.myroom.is_some() {
        if state.retail_image.is_some() {
            state.retail_image = None;
            state.retail_aabb = None;
            state.retail_status =
                RetailStatus::Failed("inside the Mog House (TopDown fallback)".into());
        }
        return;
    }

    if state.retail_image.is_some() && state.retail_zone == Some(zone_id) {
        return;
    }

    if state.retail_image.is_some() {
        state.retail_image = None;
        state.retail_aabb = None;
    }

    if state.retail_failed_zones.contains(&zone_id) {
        return;
    }
    let Some(root) = dat_root.0.clone() else {
        state.retail_failed_zones.insert(zone_id);
        state.retail_zone = Some(zone_id);
        state.retail_status =
            RetailStatus::Failed("no DAT root configured (FFXI_DAT_PATH unset?)".into());
        return;
    };
    let Some(dll) = calib.ensure_dll(root.root()) else {
        state.retail_failed_zones.insert(zone_id);
        state.retail_zone = Some(zone_id);
        state.retail_status = RetailStatus::Failed("FFXiMain.dll not readable".into());
        return;
    };
    let Some(record) = zone_map_for_player(&dll, zone_id, Some(player_map_xz(&scene_state))) else {
        state.retail_failed_zones.insert(zone_id);
        state.retail_zone = Some(zone_id);
        state.retail_status = RetailStatus::Failed(format!(
            "zone {zone_id} ships no map in the FFXiMain zone-map table"
        ));
        return;
    };
    writer.write(LoadRetailMapRequest { record });
}

pub fn process_load_retail_map_requests(
    mut events: MessageReader<LoadRetailMapRequest>,
    dat_root: Res<MinimapDatRoot>,
    mut state: ResMut<MinimapState>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(dat_root) = dat_root.0.as_ref() else {
        return;
    };

    for req in events.read() {
        let rec = req.record;
        let Some((handle, aabb)) = load_zone_map_image(dat_root, &rec, &mut images) else {
            let why = describe_map_load_failure(dat_root, rec.file_id);
            warn!(
                "minimap/retail: zone {} map {} (file {}): {}",
                rec.zone_id, rec.sub_zone_id, rec.file_id, why
            );
            state.retail_failed_zones.insert(rec.zone_id);
            state.retail_zone = Some(rec.zone_id);
            state.retail_status = RetailStatus::Failed(why);
            continue;
        };

        info!(
            "minimap/retail: loaded zone {} map {} (file {}, {} yalms across)",
            rec.zone_id, rec.sub_zone_id, rec.file_id, rec.size,
        );

        state.retail_image = Some(handle);
        state.retail_zone = Some(rec.zone_id);
        state.retail_status = RetailStatus::Loaded;
        // Never fall back to `state.aabb`: a top-down bake's extent is a
        // different world box, and drawing the retail picture against it puts
        // every marker off the map (kuluu-bqm5).
        state.retail_aabb = Some(aabb);
    }
}

/// Why a map DAT would not decode, for the `/minimap` status line.
fn describe_map_load_failure(dat_root: &ffxi_dat::DatRoot, file_id: u32) -> String {
    let path = match dat_root.resolve(file_id) {
        Ok(loc) => loc.path_under(dat_root),
        Err(e) => return format!("file_id {file_id} unresolved in DAT tree: {e}"),
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return format!("read failed: {}: {e}", path.display()),
    };
    let flags = graphic_flags_present(&bytes);
    if flags.is_empty() {
        "no Graphic chunk found in DAT".to_string()
    } else {
        format!(
            "no decodable Graphic chunk; flags present: [{}]",
            flags.join(", ")
        )
    }
}
