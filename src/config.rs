use log::*;
use serde::Deserialize;
use std::path::PathBuf;

const ENV_PREFIX: &str = "MOODLIGHT_";

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) broker_username: String,
    pub(crate) broker_password: String,
    pub(crate) broker_host: String,
    #[serde(default = "default_mqtt_port")]
    pub(crate) broker_port: u16,
    #[serde(default = "default_blaster")]
    pub(crate) blaster: PathBuf,
    pub(crate) pin_r: u8,
    pub(crate) pin_g: u8,
    pub(crate) pin_b: u8,
    #[serde(default = "default_state_file")]
    pub(crate) state_file: PathBuf,
    #[serde(default = "default_rainbow_steps")]
    pub(crate) rainbow_steps: f32,
    #[serde(default = "default_rainbow_time")]
    pub(crate) rainbow_time: f32,
}

impl Config {
    pub(crate) fn load() -> anyhow::Result<Self> {
        let config = envy::prefixed(ENV_PREFIX).from_env::<Config>()?;
        debug!("{:?}", config);
        Ok(config)
    }
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_blaster() -> PathBuf {
    PathBuf::from("/dev/pi-blaster")
}

fn default_state_file() -> PathBuf {
    PathBuf::from("/var/moodlight_state")
}

fn default_rainbow_steps() -> f32 {
    3600.0
}

fn default_rainbow_time() -> f32 {
    60.0
}
