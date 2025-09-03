#![windows_subsystem = "windows"]

use std::{fs::File, io::Write, path::Path, sync::atomic::Ordering};

use auto_launch::AutoLaunch;
use env_logger::{Builder, Target};
use log::LevelFilter;
use win_hotkey::{HotkeyManager, HotkeyManagerImpl};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use winrt_notification::Toast;

use crate::{config::Config, render::ENABLED};

mod config;
mod render;
mod update;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_ID: &str = "ScreenFilter";

fn panic_handler(info: &std::panic::PanicHookInfo) {
    let Ok(path) = std::env::current_exe() else {
        return;
    };
    let log_path = path.with_file_name("panic.txt");
    let Ok(mut file) = File::options().append(true).create(true).open(log_path) else {
        return;
    };
    let _ = writeln!(file, "Panic occurred: {}", info);
}

fn configure_auto_launch(config: &Config, path: &Path) {
    let auto = AutoLaunch::new(APP_ID, path.to_str().unwrap(), &[] as &[&str]);
    if config.launch_on_startup {
        log::info!("Enabling launch on startup");
        auto.enable().unwrap();
    } else {
        log::info!("Disabling launch on startup");
        auto.disable().unwrap();
    }
}

fn configure_hotkey(config: &Config) -> Result<HotkeyManager<()>, Box<dyn std::error::Error>> {
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
            return Err(Box::new(err));
        }
    };

    Ok(mgr)
}

fn register_app_id() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (classes, _) = hkcu
        .create_subkey("SOFTWARE\\Classes\\AppUserModelId")
        .unwrap();
    let (appkey, _) = classes.create_subkey(APP_ID).unwrap();

    appkey.set_value("DisplayName", &"Screen Filter").unwrap();
}

fn main() {
    std::panic::set_hook(Box::new(panic_handler));

    register_app_id();

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

    match update::check_for_updates() {
        Ok(Some(release)) => {
            log::info!(
                "Update available: {}, download at {}",
                release.tag_name,
                release.html_url
            );
            Toast::new(APP_ID)
                .title("Screen Filter Update Available")
                .text1(&format!(
                    "A new version of Screen Filter is available: {}, goto {} to download.",
                    release.tag_name, release.html_url
                ))
                .show()
                .unwrap();
        }
        Ok(None) => {
            log::info!("No updates available");
        }
        Err(err) => {
            log::error!("Failed to check for updates: {}", err);
            Toast::new(APP_ID)
                .title("Screen Filter Update Check Failed")
                .text1(&format!("Failed to check for updates: {}", err))
                .show()
                .unwrap();
        }
    }

    let config = config::get_config();

    configure_auto_launch(&config, &path);

    let fragment = config.mode.fragment_shader();

    std::thread::spawn(|| {
        log::info!("Starting render loop");
        match render::render_loop(fragment) {
            Ok(_) => {}
            Err(err) => {
                log::error!("Render loop error: {}", err);
                Toast::new(APP_ID)
                    .title("Screen Filter Error")
                    .text1(&format!("Render loop error: {}", err))
                    .show()
                    .unwrap();
            }
        };
    });

    let mgr = configure_hotkey(&config);
    match mgr {
        Ok(mgr) => {
            mgr.event_loop();
        }
        Err(err) => {
            log::error!("Hotkey manager error: {}", err);
            Toast::new(APP_ID)
                .title("Screen Filter Error")
                .text1(&format!("Hotkey manager error: {}", err))
                .show()
                .unwrap();
        }
    }
}
