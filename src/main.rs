#![windows_subsystem = "windows"]

use std::sync::atomic::Ordering;

use auto_launch::AutoLaunch;
use win_hotkey::{HotkeyManager, HotkeyManagerImpl};

use crate::render::ENABLED;

mod config;
mod render;

fn main() {
    let config = config::get_config();

    const APP_NAME: &str = "Screen Filter";
    let path = std::env::current_exe().unwrap();
    let auto = AutoLaunch::new(APP_NAME, path.to_str().unwrap(), &[] as &[&str]);
    if config.launch_on_startup {
        auto.enable().unwrap();
    } else {
        auto.disable().unwrap();
    }

    let fragment = config.mode.fragment_shader();

    std::thread::spawn(|| {
        match render::render_loop(fragment) {
            Ok(_) => {}
            Err(err) => println!("Render loop error: {}", err),
        };
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
