#![windows_subsystem = "windows"]

use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::{Arc, atomic::Ordering},
    thread::JoinHandle,
};

use auto_launch::AutoLaunch;
use crossbeam_channel::Sender;
use env_logger::{Builder, Target};
use log::LevelFilter;
use win_hotkey::{HotkeyManager, HotkeyManagerImpl, InterruptHandle};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use winrt_notification::Toast;

use crate::{config::Config, render::FilterControl};

mod color;
mod config;
mod gpu;
mod render;
mod update;
mod window;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_ID: &str = "ScreenFilter";

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

fn panic_handler(info: &std::panic::PanicHookInfo) {
    if let Ok(path) = std::env::current_exe()
        && let Ok(mut file) = File::options()
            .append(true)
            .create(true)
            .open(path.with_file_name("panic.txt"))
    {
        let _ = writeln!(file, "{info}");
    }
}

fn notify(title: &str, message: &str) {
    if let Err(error) = Toast::new(APP_ID).title(title).text1(message).show() {
        log::warn!("Notification failed: {error}");
    }
}

fn configure_auto_launch(config: &Config, path: &Path) -> AppResult<()> {
    let auto = AutoLaunch::new(
        APP_ID,
        path.to_str().ok_or("Executable path requires Unicode")?,
        &[] as &[&str],
    );
    if config.launch_on_startup {
        auto.enable()?;
    } else {
        auto.disable()?;
    }
    Ok(())
}

fn configure_hotkeys(config: &Config, control: Arc<FilterControl>) -> AppResult<HotkeyManager<()>> {
    let mut manager = HotkeyManager::new();
    let toggle_control = Arc::clone(&control);
    manager.register(
        config.toggle.vk,
        Some(&config.toggle.mods),
        Some(move || {
            toggle_control.enabled.fetch_xor(true, Ordering::Relaxed);
        }),
    )?;
    manager.register(
        config.freeze.vk,
        Some(&config.freeze.mods),
        Some(move || {
            control.frozen.fetch_xor(true, Ordering::Relaxed);
        }),
    )?;
    Ok(manager)
}

fn register_app_id() -> AppResult<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (appkey, _) = hkcu.create_subkey(format!("SOFTWARE\\Classes\\AppUserModelId\\{APP_ID}"))?;
    appkey.set_value("DisplayName", &"Screen Filter")?;
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(panic_handler));
    if let Err(error) = run() {
        log::error!("Application error: {error}");
        notify("Screen Filter stopped", &error.to_string());
    }
}

fn run() -> AppResult<()> {
    let path = std::env::current_exe()?;
    let log_target = File::options()
        .append(true)
        .create(true)
        .open(path.with_file_name("log.txt"))?;
    Builder::new()
        .target(Target::Pipe(Box::new(log_target)))
        .filter(None, LevelFilter::Info)
        .try_init()?;
    register_app_id()?;

    std::thread::spawn(|| match update::check_for_updates() {
        Ok(Some(release)) => {
            log::info!(
                "Update available: {} at {}",
                release.tag_name,
                release.html_url
            );
            notify(
                "Screen Filter update available",
                &format!("{}: {}", release.tag_name, release.html_url),
            );
        }
        Ok(None) => log::info!("Running the current release"),
        Err(error) => log::warn!("Update check failed: {error}"),
    });

    let config_receiver = config::watch_config(&path)?;
    let mut active = None::<EventLoop>;
    let mut last_config = None;
    let control = Arc::new(FilterControl::default());

    while let Ok(config) = config_receiver.recv() {
        if last_config.as_ref() == Some(&config) {
            continue;
        }
        // Joining the old workers releases their hotkeys and capture resources before restart.
        drop(active.take());
        if let Err(error) = configure_auto_launch(&config, &path) {
            log::warn!("Startup registration failed: {error}");
        }
        match EventLoop::start(&config, Arc::clone(&control)) {
            Ok(event_loop) => {
                active = Some(event_loop);
                last_config = Some(config);
                notify("Screen Filter ready", "Hotkeys are active.");
            }
            Err(error) => {
                log::error!("Event loop startup failed: {error}");
                notify("Screen Filter paused", &error.to_string());
                last_config = None;
            }
        }
    }
    Ok(())
}

struct EventLoop {
    stop: Sender<()>,
    interrupt: InterruptHandle,
    render_thread: Option<JoinHandle<()>>,
    hotkey_thread: Option<JoinHandle<()>>,
}

impl EventLoop {
    fn start(config: &Config, control: Arc<FilterControl>) -> AppResult<Self> {
        let manager = configure_hotkeys(config, Arc::clone(&control))?;
        let interrupt = manager.interrupt_handle();
        let (stop, receiver) = crossbeam_channel::bounded(1);
        let fragment = config.mode.fragment_shader();
        let refresh_rate = config.refresh_rate;
        let render_thread = std::thread::spawn(move || {
            if let Err(error) = render::render_loop(fragment, refresh_rate, receiver, control) {
                log::error!("Render loop failed: {error}");
                notify("Screen Filter paused", &error.to_string());
            }
        });
        let hotkey_thread = std::thread::spawn(move || manager.event_loop());
        Ok(Self {
            stop,
            interrupt,
            render_thread: Some(render_thread),
            hotkey_thread: Some(hotkey_thread),
        })
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        self.interrupt.interrupt();
        for handle in [&mut self.render_thread, &mut self.hotkey_thread] {
            if let Some(handle) = handle.take()
                && handle.join().is_err()
            {
                log::error!("Worker thread panicked during shutdown");
            }
        }
    }
}
