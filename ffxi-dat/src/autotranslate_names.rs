use std::path::Path;

use ffxi_proto::autotranslate::NameResolver;

use crate::archive::DatRoot;
use crate::item_dat::ItemTable;
use crate::key_item::KeyItemTable;

const UNUSED_ITEM_NAME: &str = ".";

#[derive(Default)]
pub struct InstalledNames {
    items: ItemTable,
    key_items: KeyItemTable,
}

impl InstalledNames {
    pub fn open_from_root(root: &DatRoot) -> InstalledNames {
        InstalledNames {
            items: ItemTable::open_from_root(root),
            key_items: KeyItemTable::open_from_root(root),
        }
    }

    pub fn open(root_dir: &Path) -> InstalledNames {
        InstalledNames {
            items: ItemTable::open(root_dir),
            key_items: KeyItemTable::open(root_dir),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.key_items.is_empty()
    }
}

impl NameResolver for InstalledNames {
    fn item_name(&self, item_id: u16) -> Option<String> {
        self.items
            .name(item_id)
            .filter(|name| !name.is_empty() && name != UNUSED_ITEM_NAME)
    }

    fn key_item_name(&self, key_item_id: u16) -> Option<String> {
        self.key_items.lookup(key_item_id).map(str::to_string)
    }
}
