use std::time::Duration;

use log::*;
use palette::{encoding, rgb::Rgb, FromColor, Hsv};
use rumqttc::v5::{mqttbytes::QoS, AsyncClient};
use serde::{de::Visitor, Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

use crate::{Color, Config, ControlMessage, OnState};

const MIN_RAINBOW_SPEED_S: f32 = 1.0;
const MAX_RAINBOW_SPEED_S: f32 = 60.0;
const MAX_RAINBOW_SPEED_SETTING: f32 = 100.0; // the min is always assumed to be 0

// since the minimum speed setting means maximum speed time, calculate a slope to map the range
// 0..MAX_RAINBOW_SPEED_SETTING to MAX_RAINBOW_SPEED_S..MIN_RAINBOW_SPEED_S (note the inversed min and max). since the
// ranges are "inversed", the slope is negative
const RAINBOW_SPEED_SLOPE: f32 = (MIN_RAINBOW_SPEED_S - MAX_RAINBOW_SPEED_S) / MAX_RAINBOW_SPEED_SETTING;

const TRANSITION_LENGTH_S: f32 = 0.5;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Static,
    Rainbow,
}

#[derive(Debug)]
struct HsColorMode;

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    pub color: Color,
    pub brightness: u8,
    pub rainbow_speed: f32,
    pub mode: Mode,
    pub state: OnState,

    color_mode: HsColorMode,
    #[serde(skip)]
    transition: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            color: Color { h: 360.0, s: 100.0 },
            brightness: u8::MAX,
            rainbow_speed: MAX_RAINBOW_SPEED_S,
            mode: Mode::Static,
            state: OnState::Off,

            color_mode: HsColorMode,
            transition: false,
        }
    }
}

impl Serialize for HsColorMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("hs")
    }
}

impl<'de> Deserialize<'de> for HsColorMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct HsColorModeVisitor;

        impl<'de> Visitor<'de> for HsColorModeVisitor {
            type Value = HsColorMode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("the string 'hs'")
            }

            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v == "hs" {
                    Ok(HsColorMode)
                } else {
                    Err(serde::de::Error::invalid_value(serde::de::Unexpected::Str(v), &self))
                }
            }
        }

        deserializer.deserialize_str(HsColorModeVisitor)
    }
}

impl State {
    pub fn edit(&mut self, msg: ControlMessage) {
        *self = Self {
            color: match (self.mode, msg.mode) {
                // update the colour only if the current mode is static, or it's being set to static
                (Mode::Static, _) | (_, Some(Mode::Static)) => msg.color.unwrap_or(self.color),
                _ => self.color,
            },
            brightness: msg.brightness.unwrap_or(self.brightness),
            rainbow_speed: msg
                .rainbow_speed
                .map(|s| s.clamp(0., MAX_RAINBOW_SPEED_SETTING))
                .unwrap_or(self.rainbow_speed),
            state: msg.state.unwrap_or(self.state),
            mode: msg.mode.unwrap_or(self.mode),

            transition: msg.state.map_or(false, |state| state != self.state),
            color_mode: HsColorMode,
        };
    }

    pub async fn publish_to_mqtt(&self, client: &AsyncClient, state_topic: &str) -> anyhow::Result<()> {
        let state_json = serde_json::to_vec(self).expect("failed to serialise state");

        if let Err(e) = client.publish(state_topic, QoS::AtLeastOnce, true, state_json).await {
            error!("Failed to publish current state: {e}");
        }

        Ok(())
    }

    pub fn step_hue(&mut self, step_duration: f32) {
        // the rainbow speed is a measure of how long it should take to go through all the colours, i.e. go through the
        // 360 degrees of the colour wheel. the value is between 0 and 100 where 0 = slowest, i.e. longest time and 100
        // = fastest, i.e. quickest time. the slope constant provides this mapping. by knowing how often the steps are
        // taken, calculate how long each step should be to achieve the correct time

        // the maximum speed is the start of the range. since the slope is negative, this will decrease the time as the
        // speed increases
        let rainbow_time = MAX_RAINBOW_SPEED_S + RAINBOW_SPEED_SLOPE * self.rainbow_speed;
        let steps_in_time = rainbow_time / step_duration;
        let step_size = 360.0 / steps_in_time;

        self.color = Color {
            h: (self.color.h + step_size) % 360.0,
            ..self.color
        };
    }

    pub async fn apply(&mut self, config: &Config) -> anyhow::Result<()> {
        if self.transition {
            self.apply_transition(config).await
        } else {
            self.apply_immediate(config).await
        }
    }

    async fn apply_immediate(&self, config: &Config) -> anyhow::Result<()> {
        let hsv = if self.state == OnState::On {
            Hsv::new(self.color.h, self.color.s / 100.0, self.brightness as f32 / 255.0)
        } else {
            Hsv::default()
        };

        write_hsv_to_blaster(hsv, config).await
    }

    async fn apply_transition(&mut self, config: &Config) -> anyhow::Result<()> {
        self.transition = false;

        let initial_brightness = self.brightness as f32 / 255.;
        let steps_in_time = TRANSITION_LENGTH_S / config.step_duration;
        let step_size = initial_brightness / steps_in_time;
        let sleep_duration = Duration::from_secs_f32(config.step_duration);

        let (mut current_brightness, target_brightness, step_size) = if self.state == OnState::On {
            // current state on means we're transitioning from off to on, and have to use a positive step
            (0., initial_brightness, step_size)
        } else {
            // current state off means we're transitioning from on to off, and have to use a negative step
            (initial_brightness, 0., -step_size)
        };

        let brightness_range_end = target_brightness.max(initial_brightness);

        debug!(
            "Transitioning to {:?}. {current_brightness} -> {target_brightness} in {steps_in_time} steps of \
             {step_size} over {TRANSITION_LENGTH_S}s",
            self.state
        );

        // apply the current first brightness since the loop steps the brightness before applying
        let hsv = Hsv::new(self.color.h, self.color.s / 100.0, current_brightness);
        write_hsv_to_blaster(hsv, config).await?;

        loop {
            current_brightness += step_size;
            debug!("{current_brightness}");

            let hsv = Hsv::new(
                self.color.h,
                self.color.s / 100.0,
                // clamp the brightness value between 0 and the larger of the target brightness (going up) or the
                // initial brightness (going down). the clamp is set here instead of to the brightness value directly
                // to ensure the last iteration step takes it outside the transition brightness range and the loop
                // terminates
                current_brightness.clamp(0., brightness_range_end),
            );

            write_hsv_to_blaster(hsv, config).await?;

            if !(0.0..brightness_range_end).contains(&current_brightness) {
                break;
            }

            tokio::time::sleep(sleep_duration).await;
        }

        debug!("Transition complete");
        Ok(())
    }
}

async fn write_hsv_to_blaster(hsv: Hsv<encoding::Srgb, f32>, config: &Config) -> anyhow::Result<()> {
    let rgb = Rgb::from_color(hsv);
    let msg = format!(
        "{pin_r}={r} {pin_g}={g} {pin_b}={b}\n",
        pin_r = config.pin_r,
        pin_g = config.pin_g,
        pin_b = config.pin_b,
        r = rgb.red,
        g = rgb.green,
        b = rgb.blue
    );

    debug!(
        "Writing to blaster: {hsv:?} -> {rgb:?} -> \"{}\"",
        &msg[..msg.len() - 1],
    );

    let mut blaster = OpenOptions::new()
        .read(false)
        .write(true)
        .create(false)
        .open(&config.blaster)
        .await?;

    blaster.write_all(msg.as_bytes()).await?;
    Ok(())
}
