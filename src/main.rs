use gethostname::gethostname;
use log::*;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, Publish, QoS};
use serde::Deserialize;
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

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
    blaster: String,
    pin_r: u8,
    pin_g: u8,
    pin_b: u8,
}

#[derive(Debug, Deserialize)]
struct ControlMessage {
    #[serde(default)]
    r: Option<u8>,
    #[serde(default)]
    g: Option<u8>,
    #[serde(default)]
    b: Option<u8>,
    #[serde(default)]
    on: Option<bool>,
}

#[derive(Debug, Default)]
struct State {
    r: u8,
    g: u8,
    b: u8,
    on: bool,
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_blaster() -> String {
    String::from("/dev/pi-blaster")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = envy::prefixed(ENV_PREFIX).from_env::<Config>()?;
    debug!("{:?}", config);

    let mut mqtt_options = MqttOptions::new(
        gethostname().to_string_lossy(),
        &config.broker_host,
        config.broker_port,
    );
    mqtt_options
        .set_credentials(&config.broker_username, &config.broker_password)
        .set_keep_alive(10);

    let (client, mut eventloop) = AsyncClient::new(mqtt_options, 10);
    client.subscribe(MQTT_TOPIC, QoS::AtLeastOnce).await?;

    let mut state = State::default();
    info!("Applying default state: {:?}", state);
    state.apply(&config).await?;

    loop {
        match eventloop.poll().await? {
            Event::Incoming(Packet::ConnAck(ack)) => info!("Connected to broker ({:?})", ack),
            Event::Incoming(Packet::SubAck(ack)) => info!("Subscribed to topic ({:?})", ack),
            Event::Incoming(Packet::Publish(Publish { payload, .. })) => {
                debug!("Payload: {:?}", payload);

                match process_control_message(&payload, &mut state, &config).await {
                    Ok(()) => info!("Control message processed. Current state: {:?}", state),
                    Err(e) => error!("Control message processing failed: {:?}", e),
                }
            }

            e => debug!("Unhandled event: {:?}", e),
        }
    }
}

async fn process_control_message(
    payload: &[u8],
    state: &mut State,
    config: &Config,
) -> anyhow::Result<()> {
    let msg = serde_json::from_slice::<ControlMessage>(payload)?;
    info!("Received control message: {:?}", msg);

    state.edit(msg);
    state.apply(config).await
}

impl State {
    fn edit(&mut self, msg: ControlMessage) {
        *self = Self {
            r: msg.r.unwrap_or(self.r),
            g: msg.g.unwrap_or(self.g),
            b: msg.b.unwrap_or(self.b),
            on: msg.on.unwrap_or(self.on),
        };
    }

    async fn apply(&self, config: &Config) -> anyhow::Result<()> {
        let (r, g, b) = if self.on {
            (
                self.r as f32 / 255.0,
                self.g as f32 / 255.0,
                self.b as f32 / 255.0,
            )
        } else {
            (0.0, 0.0, 0.0)
        };

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
            config.blaster
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