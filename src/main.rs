#![windows_subsystem = "windows"]

use std::{fs::File, io::Write, sync::atomic::Ordering};

use auto_launch::AutoLaunch;
use env_logger::{Builder, Target};
use log::LevelFilter;
use win_hotkey::{HotkeyManager, HotkeyManagerImpl};

use crate::render::ENABLED;

mod config;
mod render;

fn panic_handler(info: &std::panic::PanicHookInfo) {
    let Ok(path) = std::env::current_exe() else {
        return;
    };
    let log_path = path.with_file_name("panic.txt");
    let Ok(mut file) = File::options().append(true).create(true).open(log_path) else {
        return;
    };
    writeln!(file, "Panic occurred: {}", info).unwrap();
}

fn main() {
    std::panic::set_hook(Box::new(panic_handler));

    const APP_NAME: &str = "Screen Filter";
    let path = std::env::current_exe().unwrap();
    let log_target = Box::new(
        File::options()
            .append(true)
            .create(true)
            .open(path.with_file_name("log.txt"))
            .unwrap(),
    );
    Builder::new()
        .target(Target::Pipe(log_target))
        .filter(None, LevelFilter::Info)
        .init();

    let config = config::get_config();

    let auto = AutoLaunch::new(APP_NAME, path.to_str().unwrap(), &[] as &[&str]);
    if config.launch_on_startup {
        log::info!("Enabling launch on startup");
        auto.enable().unwrap();
    } else {
        log::info!("Disabling launch on startup");
        auto.disable().unwrap();
    }

    let fragment = config.mode.fragment_shader();

    std::thread::spawn(|| {
        log::info!("Starting render loop");
        match render::render_loop(fragment) {
            Ok(_) => {}
            Err(err) => log::error!("Render loop error: {}", err),
        };
    });

    let (vk, mods) = config.parse_hotkey().unwrap();
    let mut mgr = HotkeyManager::new();
    match mgr.register(
        vk,
        Some(&mods),
        Some(move || {
            ENABLED.fetch_xor(true, Ordering::Relaxed);
        }),
    ) {
        Ok(_) => {
            log::info!("Hotkey registered, entering event loop");
        }
        Err(err) => {
            log::error!("Failed to register hotkey: {}", err);
            return;
        }
    };

    mgr.event_loop();
}
