use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use serde::Deserialize;
use win_hotkey::keys::{ModifiersKey, VirtualKey};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum FilterMode {
    OklabGrayscale,
    LabGrayscale,
}

impl FilterMode {
    pub fn fragment_shader(self) -> &'static str {
        match self {
            Self::OklabGrayscale => include_str!("shaders/oklab_grayscale.hlsl"),
            Self::LabGrayscale => include_str!("shaders/lab_grayscale.hlsl"),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct KeySequence {
    pub vk: VirtualKey,
    pub mods: Vec<ModifiersKey>,
}

impl KeySequence {
    fn modifiers(&self) -> u32 {
        self.mods
            .iter()
            .fold(0, |bits, key| bits | key.to_mod_code())
    }
}

impl<'de> Deserialize<'de> for KeySequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let mut vk = None;
        let mut mods = Vec::new();

        for token in value.split('+').map(str::trim) {
            if token.len() == 1 {
                if vk.is_some() {
                    return Err(serde::de::Error::custom("Use exactly one hotkey character"));
                }
                vk = Some(
                    VirtualKey::from_char(token.chars().next().expect("one-byte token"))
                        .map_err(serde::de::Error::custom)?,
                );
            } else {
                let modifier =
                    ModifiersKey::from_keyname(token).map_err(serde::de::Error::custom)?;
                if matches!(modifier, ModifiersKey::Non | ModifiersKey::NoRepeat)
                    || mods.contains(&modifier)
                {
                    return Err(serde::de::Error::custom(
                        "Use distinct ctrl, alt, shift or win modifiers",
                    ));
                }
                mods.push(modifier);
            }
        }

        Ok(Self {
            vk: vk.ok_or_else(|| serde::de::Error::custom("A hotkey character is required"))?,
            mods,
        })
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub toggle: KeySequence,
    pub freeze: KeySequence,
    pub mode: FilterMode,
    pub launch_on_startup: bool,
    pub refresh_rate: u32,
}

impl Config {
    pub fn parse(source: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config = toml::from_str::<Self>(source)?;
        if config.toggle.vk == config.freeze.vk
            && config.toggle.modifiers() == config.freeze.modifiers()
        {
            return Err("Toggle and freeze require distinct hotkeys".into());
        }
        Ok(config)
    }
}

const CONFIG_FILE: &str = "config.toml";
const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

pub fn watch_config(executable: &Path) -> Result<Receiver<Config>, Box<dyn std::error::Error>> {
    let config_path = executable.with_file_name(CONFIG_FILE);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
    {
        Ok(mut file) => file.write_all(DEFAULT_CONFIG.as_bytes())?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }

    let (change_tx, change_rx) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), change_tx)?;
    // Watching the directory also handles editors that replace the file atomically.
    debouncer.watcher().watch(
        config_path.parent().ok_or("Config directory is required")?,
        RecursiveMode::NonRecursive,
    )?;

    let initial = Config::parse(&fs::read_to_string(&config_path)?)?;
    let (config_tx, config_rx) = mpsc::channel();
    config_tx.send(initial)?;

    std::thread::Builder::new()
        .name("config".into())
        .spawn(move || {
            let _debouncer = debouncer;
            while let Ok(event) = change_rx.recv() {
                match event {
                    Ok(events) if events.iter().any(|event| event.path == config_path) => {}
                    Ok(_) => continue,
                    Err(error) => {
                        log::error!("Config watch error: {error}");
                        continue;
                    }
                }

                let config = fs::read_to_string(&config_path)
                    .map_err(|error| error.to_string())
                    .and_then(|source| Config::parse(&source).map_err(|error| error.to_string()));
                match config {
                    Ok(config) => {
                        if config_tx.send(config).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        log::error!("Config reload failed, retaining active settings: {error}")
                    }
                }
            }
        })?;

    Ok(config_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = Config::parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(config.mode, FilterMode::OklabGrayscale);
        assert_eq!(config.refresh_rate, 0);
    }

    #[test]
    fn hotkey_requires_one_character_and_distinct_modifiers() {
        for key in [
            "ctrl",
            "ctrl+a+b",
            "ctrl+control+c",
            "ctrl++c",
            "non+c",
            "norepeat+c",
        ] {
            let source = DEFAULT_CONFIG.replace("ctrl+alt+c", key);
            assert!(Config::parse(&source).is_err(), "{key}");
        }
    }

    #[test]
    fn hotkey_spacing_and_case_are_accepted() {
        let source = DEFAULT_CONFIG.replace("ctrl+alt+c", " CTRL + Alt + C ");
        assert!(Config::parse(&source).is_ok());
    }

    #[test]
    fn duplicate_hotkeys_are_rejected_regardless_of_modifier_order() {
        let source = DEFAULT_CONFIG.replace("ctrl+alt+f", "alt+ctrl+c");
        assert!(Config::parse(&source).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(Config::parse(&format!("{DEFAULT_CONFIG}\nrefresh_rates = 60")).is_err());
    }
}
