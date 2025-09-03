use std::{sync::mpsc::Receiver, time::Duration};

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use serde::Deserialize;
use win_hotkey::keys::{ModifiersKey, VirtualKey};

#[derive(Deserialize, Debug, Clone, Copy)]
pub enum FilterMode {
    OklabGrayscale,
    LabGrayscale,
}

const OKLAB_GRAYSCALE_FRAGMENT_SHADER: &str = include_str!("./shaders/oklab_grayscale.hlsl");
const LAB_GRAYSCALE_FRAGMENT_SHADER: &str = include_str!("./shaders/lab_grayscale.hlsl");

impl FilterMode {
    pub fn fragment_shader(&self) -> &'static str {
        match self {
            FilterMode::OklabGrayscale => OKLAB_GRAYSCALE_FRAGMENT_SHADER,
            FilterMode::LabGrayscale => LAB_GRAYSCALE_FRAGMENT_SHADER,
        }
    }
}

pub struct KeySequence {
    pub vk: VirtualKey,
    pub mods: Vec<ModifiersKey>,
}

impl<'de> Deserialize<'de> for KeySequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let mut vk = None;
        let mut mods = Vec::new();

        for token in s.split('+') {
            if token.len() == 1 {
                vk = Some(
                    VirtualKey::from_char(token.chars().next().unwrap())
                        .map_err(serde::de::Error::custom)?,
                );
            } else {
                mods.push(ModifiersKey::from_keyname(token).map_err(serde::de::Error::custom)?);
            }
        }

        Ok(KeySequence {
            vk: vk.ok_or_else(|| serde::de::Error::custom("No virtual key found"))?,
            mods,
        })
    }
}

#[derive(Deserialize)]
pub struct Config {
    pub toggle: KeySequence,
    pub freeze: KeySequence,
    pub mode: FilterMode,
    pub launch_on_startup: bool,
    pub refresh_rate: u32,
}

const CONFIG_FILE: &str = "config.toml";
const DEFAULT_CONFIG: &str = include_str!("./default_config.toml");

pub fn get_config() -> Receiver<Config> {
    let (config_tx, config_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let config_path = std::env::current_exe().unwrap().with_file_name(CONFIG_FILE);

        if !config_path.exists() {
            let _ = std::fs::write(&config_path, DEFAULT_CONFIG);
        }

        let (config_change_tx, config_change_rx) = std::sync::mpsc::channel();
        let _ = config_change_tx.send(Ok(Default::default()));
        let mut debouncer = new_debouncer(Duration::from_secs(1), config_change_tx).unwrap();
        if let Err(err) = debouncer
            .watcher()
            .watch(&config_path, RecursiveMode::NonRecursive)
        {
            log::error!("Unable to watch the config file at: {}", err);
            return;
        };

        loop {
            while let Ok(_) = config_change_rx.recv() {
                log::info!("Config changed, reloading.");

                let Ok(config_str) = std::fs::read_to_string(&config_path) else {
                    log::error!("Unable to read the config file.");
                    continue;
                };

                let Ok(config) = toml::from_str(&config_str) else {
                    log::error!("Unable to parse the config file.");
                    continue;
                };

                let _ = config_tx.send(config);
            }
        }
    });

    config_rx
}
