//! Cache of retail menu UI-element sprites, keyed by (group-name, index).
//! Backed by ffxi_dat::ui_element; mirrors the item/status icon caches
//! (hud/item_dat_root.rs, hud/status_ribbon.rs).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ffxi_dat::ui_element::{ui_sprite, UiSprite};
use ffxi_dat::DatRoot;

// The four "static resource" menu UI DATs. XIM hardcodes their ROM paths
// (research/xim/.../UiResourceManager.kt:21-26 — ROM/0/13, ROM/119/51,
// ROM/280/15, ROM/324/95); these are those paths reverse-mapped through
// VTABLE/FTABLE to file ids, the version-stable handle, so an install whose
// patch level shuffles the physical ROM layout still resolves. The
// day-of-week orbs and weather element icons live in id 39542 (ROM/119/51).
pub const UI_DAT_FILE_IDS: [u32; 4] = [13, 39542, 39551, 39560];

pub fn read_ui_dats(root: &DatRoot) -> Vec<(u32, Vec<u8>)> {
    UI_DAT_FILE_IDS
        .into_iter()
        .filter_map(|id| {
            let loc = root.resolve(id).ok()?;
            let bytes = std::fs::read(loc.path_under(root)).ok()?;
            Some((id, bytes))
        })
        .collect()
}

const FRAMES_JP: &str = "menu    frames  ";
const FRAMES_US: &str = "menu    framesus";

#[derive(Resource, Default, Clone)]
pub struct UiElementDatRoot(pub Option<Arc<DatRoot>>);

#[derive(Resource, Default)]
pub struct UiElementAtlas {
    dats: Vec<Arc<Vec<u8>>>,
    loaded: bool,
    unavailable: bool,
    sprites: HashMap<(String, usize), Option<Handle<Image>>>,
}

impl UiElementAtlas {
    pub fn ensure(
        &mut self,
        group: &str,
        index: usize,
        dat_root: &UiElementDatRoot,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = (group.to_string(), index);
        if let Some(slot) = self.sprites.get(&key) {
            return slot.clone();
        }
        let handle = self
            .ensure_dats(dat_root)
            .iter()
            .find_map(|bytes| resolve_sprite(bytes, group, index))
            .map(|sprite| upload_sprite(sprite, images));
        self.sprites.insert(key, handle.clone());
        handle
    }

    fn ensure_dats(&mut self, dat_root: &UiElementDatRoot) -> &[Arc<Vec<u8>>] {
        if self.loaded || self.unavailable {
            return &self.dats;
        }
        let Some(root) = dat_root.0.as_ref() else {
            self.unavailable = true;
            return &self.dats;
        };
        self.dats = read_ui_dats(root)
            .into_iter()
            .map(|(_, bytes)| Arc::new(bytes))
            .collect();
        self.loaded = true;
        self.unavailable = self.dats.is_empty();
        &self.dats
    }
}

// HorizonXI/US ships "menu    framesus" where the JP client uses
// "menu    frames  "; XIM aliases the two (UiResourceManager.kt:60-62).
fn resolve_sprite(bytes: &[u8], group: &str, index: usize) -> Option<UiSprite> {
    ui_sprite(bytes, group, index).or_else(|| {
        if group == FRAMES_JP {
            ui_sprite(bytes, FRAMES_US, index)
        } else {
            None
        }
    })
}

fn upload_sprite(sprite: UiSprite, images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new(
        Extent3d {
            width: sprite.width,
            height: sprite.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        sprite.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    images.add(image)
}

pub struct UiElementAtlasPlugin;

impl Plugin for UiElementAtlasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiElementDatRoot>()
            .init_resource::<UiElementAtlas>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_dat_root() -> Option<UiElementDatRoot> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(ffxi_dat::archive::DEFAULT_INSTALL_DIR);
        if !root.join("VTABLE.DAT").exists() {
            return None;
        }
        let root = DatRoot::open(root).ok()?;
        Some(UiElementDatRoot(Some(Arc::new(root))))
    }

    // Gated on a retail install (self-skips). Exercises the whole viewer-side
    // path against real data: load the UI DATs, resolve the frames->framesus
    // alias, decode + crop, and upload a 14x14 day-orb image into Assets.
    #[test]
    fn real_dat_day_orb_uploads_14x14() {
        let Some(dat_root) = test_dat_root() else {
            return;
        };
        let mut images = Assets::<Image>::default();
        let mut atlas = UiElementAtlas::default();

        let handle = atlas
            .ensure(FRAMES_JP, 106, &dat_root, &mut images)
            .expect("Firesday orb resolves via the frames->framesus alias");
        let image = images.get(&handle).expect("uploaded image present");
        assert_eq!(image.width(), 14);
        assert_eq!(image.height(), 14);

        // Second lookup is served from the cache (same handle).
        let again = atlas.ensure(FRAMES_JP, 106, &dat_root, &mut images);
        assert_eq!(again.as_ref(), Some(&handle));
    }

    // Gated on a retail install (self-skips). The weather-icon widget resolves
    // "font    usgaiji " 0-7 (the eight element icons, ROM/119/51.DAT); pin
    // that every index uploads as a 16x16 sprite.
    #[test]
    fn real_dat_usgaiji_weather_icons_upload_16x16() {
        let Some(dat_root) = test_dat_root() else {
            return;
        };
        let mut images = Assets::<Image>::default();
        let mut atlas = UiElementAtlas::default();

        for index in 0..8 {
            let handle = atlas
                .ensure("font    usgaiji ", index, &dat_root, &mut images)
                .unwrap_or_else(|| panic!("usgaiji element {index} resolves"));
            let image = images.get(&handle).expect("uploaded image present");
            assert_eq!((image.width(), image.height()), (16, 16), "index {index}");
        }
    }
}
