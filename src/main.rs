#![feature(derive_default_enum)]

use gethostname::gethostname;
use log::*;
use palette::{encoding, rgb::Rgb, FromColor, Hsv, RgbHue};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, Publish, QoS};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
    time::sleep,
};

const ENV_PREFIX: &str = "MOODLIGHT_";
const MQTT_TOPIC: &str = "moodlight";

#[derive(Debug, Deserialize)]
struct Config {
    broker_username: String,
    broker_password: String,
    broker_host: String,
    #[serde(default = "default_mqtt_port")]
    broker_port: u16,
    #[serde(default = "default_blaster")]
    blaster: PathBuf,
    pin_r: u8,
    pin_g: u8,
    pin_b: u8,
    #[serde(default = "default_state_file")]
    state_file: PathBuf,
    #[serde(default = "default_rainbow_steps")]
    rainbow_steps: f32,
    #[serde(default = "default_rainbow_time")]
    rainbow_time: f32,
}

#[derive(Debug, Deserialize)]
struct ControlMessage {
    #[serde(default)]
    h: Option<f32>,
    #[serde(default)]
    s: Option<f32>,
    #[serde(default)]
    v: Option<f32>,
    #[serde(default)]
    on: Option<bool>,
    #[serde(default)]
    mode: Option<Mode>,
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

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Static,
    Rainbow,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    colour: Hsv<encoding::Srgb, f32>,
    mode: Mode,
    on: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = Arc::new(envy::prefixed(ENV_PREFIX).from_env::<Config>()?);
    debug!("{:?}", config);

    let mut mqtt_options = MqttOptions::new(gethostname().to_string_lossy(), &config.broker_host, config.broker_port);
    mqtt_options
        .set_credentials(&config.broker_username, &config.broker_password)
        .set_keep_alive(10);

    let (client, mut eventloop) = AsyncClient::new(mqtt_options, 10);
    client.subscribe(MQTT_TOPIC, QoS::AtLeastOnce).await?;

    let state = State::load(&config.state_file).await?;
    state.apply(&config).await?;
    let state = Arc::new(Mutex::new(state));

    let s = Arc::clone(&state);
    let c = Arc::clone(&config);
    let rainbow_thread = tokio::spawn(async move {
        let step_size = 360.0 / c.rainbow_steps;
        let step_duration = c.rainbow_time / c.rainbow_steps;
        let cycle_duration = Duration::from_secs_f32(step_duration);
        let mut current_hue = 0f32;

        loop {
            sleep(cycle_duration).await;

            let mut state = s.lock().await;
            if state.on && state.mode == Mode::Rainbow {
                current_hue = (current_hue + step_size) % 360.0;
                state.colour = Hsv::new(current_hue, 1.0, 1.0);
                if let Err(e) = state.apply(&c).await {
                    error!("Applying new state failed: {}", e);
                }
            }
        }
    });

    loop {
        tokio::select! {
            _ = wait_for_terminate() => break,
            event = eventloop.poll() => {
                match event? {
                    Event::Incoming(Packet::ConnAck(ack)) => info!("Connected to broker ({:?})", ack),
                    Event::Incoming(Packet::SubAck(ack)) => info!("Subscribed to topic ({:?})", ack),
                    Event::Incoming(Packet::Publish(Publish { payload, .. })) => {
                        debug!("Payload: {:?}", payload);

                        let mut state = state.lock().await;
                        match process_control_message(&payload, &mut state, &config).await {
                            Ok(()) => info!("Control message processed. Current state: {:?}", state),
                            Err(e) => error!("Control message processing failed: {:?}", e),
                        }
                    }

                    e => debug!("Unhandled event: {:?}", e),
                }
            }
        }
    }

    rainbow_thread.abort();
    Ok(())
}

async fn wait_for_terminate() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    debug!("Received SIGTERM");
    Ok(())
}

async fn process_control_message(payload: &[u8], state: &mut State, config: &Config) -> anyhow::Result<()> {
    let msg = serde_json::from_slice::<ControlMessage>(payload)?;
    info!("Received control message: {:?}", msg);

    state.edit(msg);
    state.apply(config).await?;
    state.save(&config.state_file).await
}

impl State {
    async fn load(state_file: &Path) -> anyhow::Result<Self> {
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

    async fn save(&self, state_file: &Path) -> anyhow::Result<()> {
        let mut file = fs::File::create(state_file).await?;
        let serialised = serde_json::to_string(self)?;
        debug!("Saving state to {}: {}", state_file.display(), serialised);

        file.write_all(serialised.as_bytes()).await?;
        Ok(())
    }

    fn edit(&mut self, msg: ControlMessage) {
        *self = Self {
            colour: match (msg.mode, self.mode) {
                // update the colour only if the current mode is static, or it's being set to static
                (Some(Mode::Static), _) | (_, Mode::Static) => {
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

    async fn apply(&self, config: &Config) -> anyhow::Result<()> {
        let (r, g, b) = Rgb::from_color(if self.on { self.colour } else { Hsv::default() }).into_components();
        let msg = format!(
            "{pin_r}={r} {pin_g}={g} {pin_b}={b}\n",
            pin_r = config.pin_r,
            pin_g = config.pin_g,
            pin_b = config.pin_b,
            r = r,
            g = g,
            b = b
        );

        debug!(
            "Writing message \"{}\" to {}",
            &msg[..msg.len() - 1],
            config.blaster.display()
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
