use std::path::PathBuf;

use log::*;
use serde::Deserialize;

const DEFAULT_MQTT_TOPIC: &str = "moodlight";
const DEFAULT_HOME_ASSISTANT_MQTT_TOPIC: &str = "homeassistant";
const ENV_PREFIX: &str = "MOODLIGHT_";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub name: String,
    #[serde(default = "default_mqtt_topic")]
    pub mqtt_topic: String,
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
    #[serde(default = "default_step_duration")]
    pub step_duration: f32,

    #[serde(default = "default_home_assistant_topic")]
    pub home_assistant_topic: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let config = envy::prefixed(ENV_PREFIX).from_env::<Config>()?;
        debug!("{config:?}");
        Ok(config)
    }

    pub fn own_topic(&self) -> String {
        format!("{}/{}", self.mqtt_topic, self.name)
    }

    pub fn command_topic(&self) -> String {
        format!("{}/set", self.own_topic())
    }

    pub fn state_topic(&self) -> String {
        format!("{}/state", self.own_topic())
    }

    pub fn unique_id(&self) -> String {
        format!("moodlight_{}", self.name.to_ascii_lowercase().replace(' ', "_"))
    }

    pub fn home_assistant_light_topic(&self) -> String {
        format!("{}/light/{}/config", self.home_assistant_topic, self.unique_id())
    }

    pub fn home_assistant_select_topic(&self) -> String {
        format!("{}/select/{}/config", self.home_assistant_topic, self.unique_id())
    }

    pub fn home_assistant_number_topic(&self) -> String {
        format!("{}/number/{}/config", self.home_assistant_topic, self.unique_id())
    }
}

fn default_mqtt_topic() -> String {
    String::from(DEFAULT_MQTT_TOPIC)
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_blaster() -> PathBuf {
    PathBuf::from("/dev/pi-blaster")
}

fn default_step_duration() -> f32 {
    0.02
}

fn default_home_assistant_topic() -> String {
    String::from(DEFAULT_HOME_ASSISTANT_MQTT_TOPIC)
}
