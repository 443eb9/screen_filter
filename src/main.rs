use std::sync::atomic::Ordering;

use win_hotkey::{HotkeyManager, HotkeyManagerImpl, keys::VirtualKey};

use crate::render::ENABLED;

mod config;
mod render;

fn main() {
    let config = config::get_config();
    let fragment = config.mode.fragment_shader();

    std::thread::spawn(|| {
        render::render_loop(fragment).unwrap();
    });

    let (vk, mods) = config.parse_hotkey().unwrap();
    let mut mgr = HotkeyManager::new();
    mgr.register(
        vk,
        Some(&mods),
        Some(move || {
            ENABLED.fetch_xor(true, Ordering::Relaxed);
        }),
    )
    .unwrap();

    mgr.event_loop();
}
