use crate::{Config, ControlMessage};
use log::*;
use palette::{encoding, rgb::Rgb, FromColor, Hsv, RgbHue};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mode {
    #[default]
    Static,
    Rainbow,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct State {
    pub(crate) colour: Hsv<encoding::Srgb, f32>,
    pub(crate) mode: Mode,
    pub(crate) on: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            colour: Hsv::new(0.0, 1.0, 1.0),
            mode: Default::default(),
            on: Default::default(),
        }
    }
}

impl State {
    pub(crate) async fn load(state_file: &Path) -> anyhow::Result<Self> {
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

    pub(crate) async fn save(&self, state_file: &Path) -> anyhow::Result<()> {
        let mut file = fs::File::create(state_file).await?;
        let serialised = serde_json::to_string(self)?;
        debug!("Saving state to {}: {}", state_file.display(), serialised);

        file.write_all(serialised.as_bytes()).await?;
        Ok(())
    }

    pub(crate) fn edit(&mut self, msg: ControlMessage) {
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
            on: msg.on.unwrap_or(self.on),
            mode: msg.mode.unwrap_or(self.mode),
        };
    }

    pub(crate) fn step_hue(&mut self, step: f32) {
        let hue = (self.colour.hue.to_raw_degrees() + step) % 360.0;
        self.colour = Hsv::new(hue, 1.0, 1.0);
    }

    pub(crate) async fn apply(&self, config: &Config) -> anyhow::Result<()> {
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
