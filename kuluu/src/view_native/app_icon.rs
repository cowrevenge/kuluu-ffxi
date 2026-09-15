use bevy::ecs::system::NonSendMarker;
use bevy::image::{CompressedImageFormats, ImageFormat, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::window::WindowCreated;
use bevy::winit::WINIT_WINDOWS;
use winit::window::Icon;

pub const APP_ID: &str = "io.github.jondwillis.kuluu";
const ICON_PNG: &[u8] = include_bytes!("../../assets/branding/png/kuluu-256.png");

pub fn install(app: &mut App) {
    app.add_systems(Update, set_window_icons);
}

fn decode_icon() -> Icon {
    let image = Image::from_buffer(
        ICON_PNG,
        ImageType::Format(ImageFormat::Png),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        default(),
    )
    .expect("embedded Kuluu PNG decodes");
    let size = image.texture_descriptor.size;
    Icon::from_rgba(image.data.expect("icon pixels"), size.width, size.height)
        .expect("embedded Kuluu icon is RGBA")
}

fn set_window_icons(
    _main_thread: NonSendMarker,
    mut created: MessageReader<WindowCreated>,
    mut icon: Local<Option<Icon>>,
) {
    for event in created.read() {
        let icon = icon.get_or_insert_with(decode_icon);
        WINIT_WINDOWS.with_borrow(|windows| {
            if let Some(window) = windows.get_window(event.window) {
                window.set_window_icon(Some(icon.clone()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_launcher_matches_window_identity() {
        let desktop = include_str!("../../../packaging/linux/io.github.jondwillis.kuluu.desktop");
        assert!(desktop.lines().any(|line| line == format!("Icon={APP_ID}")));
        assert!(desktop
            .lines()
            .any(|line| line == format!("StartupWMClass={APP_ID}")));
    }
}
