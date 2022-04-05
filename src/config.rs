use log::*;
use serde::Deserialize;
use std::path::PathBuf;

const ENV_PREFIX: &str = "MOODLIGHT_";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub broker_username: String,
    pub broker_password: String,
    pub broker_host: String,
    #[serde(default = "default_mqtt_port")]
    pub broker_port: u16,
    #[serde(default = "default_blaster")]
    pub blaster: PathBuf,
    pub pin_r: u8,
    pub pin_g: u8,
    pub pin_b: u8,
    #[serde(default = "default_state_file")]
    pub state_file: PathBuf,
    #[serde(default = "default_rainbow_step_duration")]
    pub rainbow_step_duration: f32,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
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

fn default_rainbow_step_duration() -> f32 {
    0.02
}
