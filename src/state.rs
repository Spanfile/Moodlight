use crate::{Config, ControlMessage};
use log::*;
use palette::{encoding, rgb::Rgb, FromColor, Hsv, RgbHue};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

// both in seconds
const MIN_RAINBOW_SPEED: f32 = 1.0;
const MAX_RAINBOW_SPEED: f32 = 60.0;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Static,
    Rainbow,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    pub colour: Hsv<encoding::Srgb, f32>,
    pub rainbow_brightness: f32,
    pub rainbow_speed: f32,
    pub mode: Mode,
    pub on: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            colour: Hsv::new(0.0, 1.0, 1.0),
            rainbow_brightness: 1.0,
            rainbow_speed: MAX_RAINBOW_SPEED,
            mode: Mode::Static,
            on: false,
        }
    }
}

impl State {
    pub async fn load(state_file: &Path) -> anyhow::Result<Self> {
        if state_file.exists() {
            let state = fs::read_to_string(state_file).await?;
            let state = match serde_json::from_str(&state) {
                Ok(s) => {
                    debug!("Using saved state from file {}: {:?}", state_file.display(), state);
                    s
                }
                Err(e) => {
                    warn!("State failed to load: {}", e);
                    warn!("Using default state");
                    Self::default()
                }
            };

            Ok(state)
        } else {
            debug!(
                "State file {} doesn't exist, returning default state",
                state_file.display(),
            );

            Ok(Self::default())
        }
    }

    pub async fn save(&self, state_file: &Path) -> anyhow::Result<()> {
        let mut file = fs::File::create(state_file).await?;
        let serialised = serde_json::to_string(self)?;
        debug!("Saving state to {}: {}", state_file.display(), serialised);

        file.write_all(serialised.as_bytes()).await?;
        Ok(())
    }

    pub fn edit(&mut self, msg: ControlMessage) {
        *self = Self {
            colour: match (self.mode, msg.mode) {
                // update the colour only if the current mode is static, or it's being set to static
                (Mode::Static, _) | (_, Some(Mode::Static)) => {
                    let (h, s, v) = self.colour.into_components();
                    Hsv::from_components((
                        msg.h.map(RgbHue::from_degrees).unwrap_or(h),
                        msg.s.unwrap_or(s),
                        msg.v.unwrap_or(v),
                    ))
                }
                _ => self.colour,
            },
            rainbow_brightness: msg
                .rainbow_brightness
                .map(|b| b as f32 / 255.0)
                .unwrap_or(self.rainbow_brightness),
            rainbow_speed: msg
                .rainbow_speed
                .map(|s| ((s as f32 / 60.0) * MAX_RAINBOW_SPEED).clamp(MIN_RAINBOW_SPEED, MAX_RAINBOW_SPEED))
                .unwrap_or(self.rainbow_speed),
            on: msg.on.unwrap_or(self.on),
            mode: msg.mode.unwrap_or(self.mode),
        };
    }

    pub fn step_hue(&mut self, step_duration: f32) {
        // the rainbow speed is a measure of how long it should take to go through all the colours, i.e. go through the
        // 360 degrees of the colour wheel. by knowing how often the steps are taken, calculate how long each step
        // should be to achieve the correct time

        let steps_in_time = self.rainbow_speed / step_duration;
        let step_size = 360.0 / steps_in_time;

        let hue = (self.colour.hue.to_raw_degrees() + step_size) % 360.0;
        self.colour = Hsv::new(hue, 1.0, self.rainbow_brightness);
    }

    pub async fn apply(&self, config: &Config) -> anyhow::Result<()> {
        let hsv = if self.on { self.colour } else { Hsv::default() };
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
            "Applying state: {:?} -> {:?} -> \"{}\"",
            hsv,
            rgb,
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
}
