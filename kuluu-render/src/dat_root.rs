use std::sync::Arc;

use bevy::prelude::Resource;
use ffxi_dat::DatRoot;

/// The one install every system reads; `None` until a first run picks one.
#[derive(Resource, Default, Clone)]
pub struct SharedDatRoot(pub Option<Arc<DatRoot>>);

impl SharedDatRoot {
    pub fn get(&self) -> Option<&Arc<DatRoot>> {
        self.0.as_ref()
    }
}
