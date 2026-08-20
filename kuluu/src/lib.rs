#![allow(clippy::type_complexity, clippy::too_many_arguments)]

pub mod launcher_store;
pub mod secret_store;

#[cfg(feature = "native-window")]
pub mod graphics_store;
#[cfg(feature = "native-window")]
pub mod keybinds_store;
#[cfg(feature = "native-window")]
pub mod marker_store;
#[cfg(feature = "native-window")]
pub mod overlay_store;
