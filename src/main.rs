#![feature(derive_default_enum)]
mod config;
mod state;

use crate::state::{Mode, State};
use config::Config;
use gethostname::gethostname;
use log::*;
use palette::Hsv;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, Publish, QoS};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::sleep};

const MQTT_TOPIC: &str = "moodlight";

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = Arc::new(Config::load()?);

    let mut mqtt_options = MqttOptions::new(gethostname().to_string_lossy(), &config.broker_host, config.broker_port);
    mqtt_options
        .set_credentials(&config.broker_username, &config.broker_password)
        .set_keep_alive(10);

    let (client, mut eventloop) = AsyncClient::new(mqtt_options, 10);
    client.subscribe(MQTT_TOPIC, QoS::AtLeastOnce).await?;

    let state = State::load(&config.state_file).await?;
    state.apply(&config).await?;
    let state = Arc::new(Mutex::new(state));

    run_rainbow_thread(Arc::clone(&state), Arc::clone(&config));

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

fn run_rainbow_thread(state: Arc<Mutex<State>>, config: Arc<Config>) {
    tokio::spawn(async move {
        let step_size = 360.0 / config.rainbow_steps;
        let step_duration = config.rainbow_time / config.rainbow_steps;
        let cycle_duration = Duration::from_secs_f32(step_duration);

        loop {
            sleep(cycle_duration).await;

            let mut state = state.lock().await;
            if state.on && state.mode == Mode::Rainbow {
                let hue = (state.colour.hue.to_raw_degrees() + step_size) % 360.0;
                state.colour = Hsv::new(hue, 1.0, 1.0);

                if let Err(e) = state.apply(&config).await {
                    error!("Applying new state failed: {}", e);
                }
            }
        }
    });
}
