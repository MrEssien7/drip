// The tray icon.

use tauri::image::Image;

pub fn tray_icon() -> Image<'static> {
    const SIZE: u32 = 32;
    Image::new_owned(crate::droplet::droplet_rgba(SIZE), SIZE, SIZE)
}
