use serde::Serialize;

use crate::{config::Config, state::MAX_RAINBOW_SPEED_SETTING};

#[derive(Debug, Serialize)]
struct HomeAssistantDevice {
    name: String,
    identifiers: String,
}

#[derive(Debug, Serialize)]
pub struct HomeAssistantLightConfig {
    name: Option<()>,
    unique_id: String,
    command_topic: String,
    state_topic: String,
    device: HomeAssistantDevice,

    schema: &'static str,
    color_mode: bool,
    brightness: bool,
    supported_color_modes: &'static [&'static str],
}

#[derive(Debug, Serialize)]
pub struct HomeAssistantSelectConfig {
    name: &'static str,
    unique_id: String,
    command_topic: String,
    state_topic: String,
    device: HomeAssistantDevice,

    options: &'static [&'static str],
    command_template: &'static str,
    value_template: &'static str,
}

#[derive(Debug, Serialize)]
pub struct HomeAssistantNumberConfig {
    name: &'static str,
    unique_id: String,
    command_topic: String,
    state_topic: String,
    device: HomeAssistantDevice,

    min: f32,
    max: f32,
    mode: &'static str,
    command_template: &'static str,
    value_template: &'static str,
}

impl HomeAssistantLightConfig {
    pub fn new(config: &Config) -> Self {
        let unique_id = config.unique_id();

        Self {
            name: None,
            unique_id: format!("{}_light", unique_id),
            command_topic: config.command_topic(),
            state_topic: config.state_topic(),
            device: HomeAssistantDevice {
                name: format!("{} moodlight", config.name),
                identifiers: unique_id,
            },

            schema: "json",
            color_mode: true,
            brightness: true,
            supported_color_modes: &["hs"],
        }
    }
}

impl HomeAssistantSelectConfig {
    pub fn new(config: &Config) -> Self {
        let unique_id = config.unique_id();

        Self {
            name: "Mode",
            unique_id: format!("{}_mode_select", unique_id),
            command_topic: config.command_topic(),
            state_topic: config.state_topic(),
            device: HomeAssistantDevice {
                name: format!("{} moodlight", config.name),
                identifiers: unique_id,
            },

            options: &["Static", "Rainbow"],
            command_template: "{\"mode\": \"{{ value }}\"}",
            value_template: "{{ value_json.mode }}",
        }
    }
}

impl HomeAssistantNumberConfig {
    pub fn new(config: &Config) -> Self {
        let unique_id = config.unique_id();

        Self {
            name: "Rainbow speed",
            unique_id: format!("{}_rainbow_speed", unique_id),
            command_topic: config.command_topic(),
            state_topic: config.state_topic(),
            device: HomeAssistantDevice {
                name: format!("{} moodlight", config.name),
                identifiers: unique_id,
            },

            min: 0.,
            max: MAX_RAINBOW_SPEED_SETTING,
            mode: "slider",
            command_template: "{\"rainbow_speed\": {{ value }}}",
            value_template: "{{ value_json.rainbow_speed }}",
        }
    }
}
