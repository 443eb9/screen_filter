#![windows_subsystem = "windows"]

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use auto_launch::AutoLaunch;
use crossbeam_channel::Sender;
use env_logger::{Builder, Target};
use log::LevelFilter;
use win_hotkey::{HotkeyManager, HotkeyManagerImpl};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use winrt_notification::Toast;

use crate::{
    config::Config,
    render::{ENABLED, FROZEN},
};

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
    let mut mgr = HotkeyManager::new();
    mgr.unregister_all()?;
    mgr.register(
        config.toggle.vk,
        Some(&config.toggle.mods),
        Some(|| {
            ENABLED.fetch_xor(true, Ordering::Relaxed);
        }),
    )?;
    mgr.register(
        config.freeze.vk,
        Some(&config.freeze.mods),
        Some(|| {
            FROZEN.fetch_xor(true, Ordering::Relaxed);
        }),
    )?;

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

    let config_receiver = config::get_config();
    let mut last_terminator: Option<Sender<()>> = None;

    while let Ok(config) = config_receiver.recv() {
        if let Some(last_terminator) = &last_terminator {
            log::info!("Terminating last event loop.");
            let _ = last_terminator.send(());
            Toast::new(APP_ID)
                .title("Screen Filter restarted.")
                .show()
                .unwrap();
        } else {
            // First run
            Toast::new(APP_ID)
                .title("Screen Filter Running")
                .show()
                .unwrap();
        }

        let config = config;
        let path = path.clone();
        log::info!("Stating event loop.");
        if let Some(terminator) = start_event_loop(config, path) {
            last_terminator = Some(terminator.tx);
        }
    }
}

struct EventLoopTerminator {
    tx: Sender<()>,
}

fn start_event_loop(config: Config, path: PathBuf) -> Option<EventLoopTerminator> {
    configure_auto_launch(&config, &path);

    let fragment = config.mode.fragment_shader();

    let (terminator_tx, terminator_rx) = crossbeam_channel::unbounded();

    let trx = terminator_rx.clone();
    std::thread::spawn(move || {
        log::info!("Starting render loop");
        if let Err(err) = render::render_loop(fragment, trx.clone()) {
            log::error!("Render loop error: {}", err);
        }
    });

    let mgr = match configure_hotkey(&config) {
        Ok(ok) => ok,
        Err(err) => {
            log::error!("Hotkey manager error: {}", err);
            return None;
        }
    };

    let interrupt_handle = mgr.interrupt_handle();
    std::thread::spawn(move || {
        if let Ok(_) = terminator_rx.recv() {
            interrupt_handle.interrupt();
        }
    });

    std::thread::spawn(move || {
        mgr.event_loop();
    });

    Some(EventLoopTerminator { tx: terminator_tx })
}
