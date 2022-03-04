mod config;
mod state;

use crate::state::{Mode, State};
use config::Config;
use gethostname::gethostname;
use log::*;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, Publish, QoS};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::{self, MissedTickBehavior};

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

    let config = Config::load()?;
    let (client, mut eventloop) = create_mqtt_client(&config).await?;
    let mut state = State::load(&config.state_file).await?;
    state.apply(&config).await?;

    let (rainbow_duration, rainbow_step_size) = get_rainbow_specs(&config);
    let mut rainbow_timer = time::interval(rainbow_duration);
    // set the missed tick behavior to Delay so when the rainbow timer should tick but doesn't, because the light is off
    // or set to Static, any missed ticks are "ignored" and it'll start ticking regularly when active again
    rainbow_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = wait_for_terminate() => break,
            _ = rainbow_timer.tick(), if state.on && state.mode == Mode::Rainbow => {
                state.step_hue(rainbow_step_size);
                state.apply(&config).await?;
            }
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                        info!("Connected to broker ({:?}), subscribing to moodlight topic...", ack);
                        client.subscribe(MQTT_TOPIC, QoS::AtLeastOnce).await?;
                    }
                    Ok(Event::Incoming(Packet::SubAck(ack))) => info!("Subscribed to topic ({:?})", ack),
                    Ok(Event::Incoming(Packet::Publish(Publish { payload, .. }))) => {
                        debug!("Payload: {:?}", payload);

                        match process_control_message(&payload, &mut state, &config).await {
                            Ok(()) => info!("Control message processed. Current state: {:?}", state),
                            Err(e) => error!("Control message processing failed: {:?}", e),
                        }
                    }

                    Ok(e) => debug!("Unhandled event: {:?}", e),
                    Err(e) => error!("MQTT client returned error: {:?}", e),
                }
            }
        }
    }

    Ok(())
}

async fn create_mqtt_client(config: &Config) -> anyhow::Result<(AsyncClient, EventLoop)> {
    let mut mqtt_options = MqttOptions::new(gethostname().to_string_lossy(), &config.broker_host, config.broker_port);
    mqtt_options
        .set_credentials(&config.broker_username, &config.broker_password)
        .set_keep_alive(Duration::from_secs(10));

    let (client, eventloop) = AsyncClient::new(mqtt_options, 10);
    Ok((client, eventloop))
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

fn get_rainbow_specs(config: &Config) -> (Duration, f32) {
    let steps_in_time = config.rainbow_time / config.rainbow_step_duration;
    let step_size = 360.0 / steps_in_time;

    debug!(
        "Rainbow: {} steps (fixed length {}s) in {}s -> {} step size",
        steps_in_time, config.rainbow_step_duration, config.rainbow_time, step_size
    );
    (Duration::from_secs_f32(config.rainbow_step_duration), step_size)
}
