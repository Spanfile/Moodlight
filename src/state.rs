use log::*;
use palette::{encoding, rgb::Rgb, FromColor, Hsv};
use serde::{de::Visitor, Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

use crate::{Color, Config, ControlMessage, OnState};

// both in seconds
pub const MIN_RAINBOW_SPEED: f32 = 1.0;
pub const MAX_RAINBOW_SPEED: f32 = 60.0;

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
}

impl Default for State {
    fn default() -> Self {
        Self {
            color: Color { h: 360.0, s: 100.0 },
            brightness: u8::MAX,
            rainbow_speed: MAX_RAINBOW_SPEED,
            mode: Mode::Static,
            state: OnState::Off,
            color_mode: HsColorMode,
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
                .map(|s| (s / 60.0 * MAX_RAINBOW_SPEED).clamp(MIN_RAINBOW_SPEED, MAX_RAINBOW_SPEED))
                .unwrap_or(self.rainbow_speed),
            state: msg.state.unwrap_or(self.state),
            mode: msg.mode.unwrap_or(self.mode),
            color_mode: HsColorMode,
        };
    }

    pub fn step_hue(&mut self, step_duration: f32) {
        // the rainbow speed is a measure of how long it should take to go through all the colours, i.e. go through the
        // 360 degrees of the colour wheel. by knowing how often the steps are taken, calculate how long each step
        // should be to achieve the correct time

        let steps_in_time = self.rainbow_speed / step_duration;
        let step_size = 360.0 / steps_in_time;

        let h = (self.color.h + step_size) % 360.0;
        self.color = Color { h, ..self.color };
    }

    pub async fn apply(&self, config: &Config) -> anyhow::Result<()> {
        let hsv = if self.state == OnState::On {
            Hsv::<encoding::Srgb, f32>::new(self.color.h, self.color.s / 100.0, self.brightness as f32 / 255.0)
        } else {
            Hsv::default()
        };

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

        debug!("Applying state: {hsv:?} -> {rgb:?} -> \"{}\"", &msg[..msg.len() - 1],);

        let mut blaster = OpenOptions::new()
            .read(false)
            .write(true)
            .create(false)
            .open(&config.blaster)
            .await?;

        blaster.write_all(msg.as_bytes()).await?;
        Ok(())
    }
}
