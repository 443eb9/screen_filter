use serde::Deserialize;
use win_hotkey::keys::{ModifiersKey, VirtualKey};

#[derive(Deserialize)]
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

#[derive(Deserialize)]
pub struct Config {
    pub hotkey: String,
    pub mode: FilterMode,
}

impl Config {
    pub fn parse_hotkey(&self) -> Result<(VirtualKey, Vec<ModifiersKey>), String> {
        let mut vk = None;
        let mut mods = Vec::new();

        for token in self.hotkey.split('+') {
            if token.len() == 1 {
                vk = Some(
                    VirtualKey::from_char(token.chars().next().unwrap())
                        .map_err(|e| e.to_string())?,
                );
            } else {
                mods.push(ModifiersKey::from_keyname(token).map_err(|e| e.to_string())?);
            }
        }

        Ok((vk.ok_or("No virtual key found")?, mods))
    }
}

const CONFIG_PATH: &str = "config.toml";
const DEFAULT_CONFIG: &str = include_str!("./default_config.toml");

pub fn get_config() -> Config {
    let config_str = std::fs::read_to_string(CONFIG_PATH).unwrap_or_else(|_| {
        std::fs::write(CONFIG_PATH, DEFAULT_CONFIG).expect("Failed to write default config");
        DEFAULT_CONFIG.to_string()
    });
    toml::from_str(&config_str).expect("Failed to parse config")
}
