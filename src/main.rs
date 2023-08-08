// because you're an idiot and never remember it, the magic incantation so this runs on a Pi Zero W is
// cross build --target=arm-unknown-linux-gnueabihf --release

mod config;
mod hass;
mod state;

use std::{task::Poll, time::Duration};

use log::*;
use rumqttc::v5::{
    mqttbytes::{
        v5::{Filter, Packet, Publish},
        QoS,
    },
    AsyncClient, Event, EventLoop, MqttOptions,
};
use serde::{Deserialize, Serialize};
use tokio::{
    signal::unix::{signal, SignalKind},
    time::{self, MissedTickBehavior},
};

use crate::{
    config::Config,
    hass::{HomeAssistantLightConfig, HomeAssistantNumberConfig, HomeAssistantSelectConfig},
    state::{Mode, State},
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
pub enum OnState {
    On,
    Off,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone)]
pub struct Color {
    pub h: f32,
    pub s: f32,
}

#[derive(Debug, Deserialize)]
pub struct ControlMessage {
    #[serde(default)]
    color: Option<Color>,
    #[serde(default)]
    brightness: Option<u8>,
    #[serde(default)]
    rainbow_speed: Option<f32>,
    #[serde(default)]
    state: Option<OnState>,
    #[serde(default)]
    mode: Option<Mode>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    if cfg!(debug_assertions) {
        dotenv::dotenv()?;
    }

    env_logger::init();

    let config = Config::load()?;
    let (client, mut eventloop) = create_mqtt_client(&config).await?;

    let mut state = State::default();
    let mut initial_state_received = false;
    let mut hass_discovery_sent = false;

    // don't apply the default state, instead let the stored state in MQTT to be read and applied later
    // state.apply(&config).await?;

    let mut rainbow_timer = time::interval(Duration::from_secs_f32(config.step_duration));
    // set the missed tick behavior to Delay so when the rainbow timer should tick but doesn't, because the light is off
    // or set to Static, any missed ticks are "ignored" and it'll start ticking regularly when active again
    rainbow_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let command_topic = config.command_topic();
    let state_topic = config.state_topic();

    loop {
        tokio::select! {
            _ = wait_for_terminate() => break,
            _ = rainbow_timer.tick(), if state.state == OnState::On && state.mode == Mode::Rainbow => {
                state.step_hue(config.step_duration);
                state.apply(&config).await?;
            }

            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                        info!("Connected to broker ({ack:?})");

                        if !hass_discovery_sent {
                            send_home_assistant_discovery(&config, &client).await?;
                            hass_discovery_sent = true;
                        }

                        // subscribe to the state topic only if we haven't yet received the initial state from there
                        subscribe_to_own_topics(&config, &client, !initial_state_received).await?;
                    }

                    Ok(Event::Incoming(Packet::SubAck(ack))) => info!("Subscribed to topic ({ack:?})"),

                    Ok(Event::Incoming(Packet::Publish(Publish { payload, topic, .. }))) => {
                        let topic = String::from_utf8(topic.to_vec()).expect("non-UTF8 topic");
                        debug!("On {topic}: {payload:?}");

                        if topic == command_topic {
                            if let Err(e) = process_command_message(&payload, &mut state, &client, &state_topic, &config).await {
                                error!("Command message processing failed: {e}");
                            } else {
                                info!("Command message processed. Current state: {state:?}");
                            }
                        } else if topic == state_topic {
                            if let Err(e) = process_state_message(&payload, &mut state, &config).await {
                                error!("State message processing failed: {e}");
                            }

                            initial_state_received = true;

                            if let Err(e) = client.unsubscribe(&state_topic).await {
                                error!("Failed to unsubscribe from state topic: {e}");
                            }
                        } else {
                            warn!("Received message in unknown topic: {topic}");
                        }
                    }

                    Ok(_e) => {
                        // debug!("Unhandled event: {_e:?}");
                    }

                    Err(e) => {
                        error!("MQTT client returned error: {e:?}");
                        // TODO: check what caused the error and break only if it's something unrecoverable
                        // break;
                    }
                }
            }
        }
    }

    info!("Shutting down; saving state to MQTT");

    if let Err(e) = state.publish_to_mqtt(&client, &state_topic).await {
        error!("Failed to save state to MQTT: {e}");
    } else {
        // the publish doesn't actually go out until we poll the event loop enough times to empty the send queue

        loop {
            let eventloop_poll = eventloop.poll();
            futures::pin_mut!(eventloop_poll);

            match futures::poll!(eventloop_poll) {
                Poll::Ready(res) => debug!("{res:?}"),
                Poll::Pending => break,
            }
        }
    }

    debug!("Shutting down");
    Ok(())
}

async fn wait_for_terminate() -> anyhow::Result<()> {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigint = signal(SignalKind::interrupt()).unwrap();

    tokio::select! {
        _ = sigterm.recv() => (),
        _ = sigint.recv() => (),
    }

    debug!("Received termination signal");
    Ok(())
}

async fn create_mqtt_client(config: &Config) -> anyhow::Result<(AsyncClient, EventLoop)> {
    let mut mqtt_options = MqttOptions::parse_url(&config.broker_url)?;
    mqtt_options
        .set_credentials(&config.broker_username, &config.broker_password)
        .set_keep_alive(Duration::from_secs(10));

    let (client, eventloop) = AsyncClient::new(mqtt_options, 10);
    Ok((client, eventloop))
}

async fn send_home_assistant_discovery(config: &Config, client: &AsyncClient) -> anyhow::Result<()> {
    let light_config = HomeAssistantLightConfig::new(config);
    let select_config = HomeAssistantSelectConfig::new(config);
    let number_config = HomeAssistantNumberConfig::new(config);

    debug!("{light_config:?}");
    debug!("{select_config:?}");
    debug!("{number_config:?}");

    let light_config_json = serde_json::to_string(&light_config).expect("failed to serialize light config");
    let select_config_json = serde_json::to_string(&select_config).expect("failed to serialize select config");
    let number_config_json = serde_json::to_string(&number_config).expect("failed to serialize number config");

    info!("Sending Home Assistant MQTT discovery messages");

    client
        .publish(
            config.home_assistant_light_topic(),
            QoS::AtLeastOnce,
            true,
            light_config_json,
        )
        .await?;

    client
        .publish(
            config.home_assistant_select_topic(),
            QoS::AtLeastOnce,
            true,
            select_config_json,
        )
        .await?;

    client
        .publish(
            config.home_assistant_number_topic(),
            QoS::AtLeastOnce,
            true,
            number_config_json,
        )
        .await?;

    Ok(())
}

async fn subscribe_to_own_topics(
    config: &Config,
    client: &AsyncClient,
    include_state_topic: bool,
) -> anyhow::Result<()> {
    info!("Subscribing to own topics under {}", config.own_topic());

    let mut topics = vec![Filter {
        path: config.command_topic(),
        qos: QoS::AtLeastOnce,
        nolocal: true,
        ..Default::default()
    }];

    if include_state_topic {
        topics.push(Filter {
            path: config.state_topic(),
            qos: QoS::AtLeastOnce,
            nolocal: true,
            ..Default::default()
        });
    }

    client.subscribe_many(topics).await?;
    Ok(())
}

async fn process_command_message(
    payload: &[u8],
    state: &mut State,
    client: &AsyncClient,
    state_topic: &str,
    config: &Config,
) -> anyhow::Result<()> {
    let msg = serde_json::from_slice::<ControlMessage>(payload)?;
    info!("Received command message: {msg:?}",);

    state.edit(msg);

    // applying the state may result in a delay due to the transition time from one on state to another. since during
    // that time we're not polling the MQTT event loop, our outgoing publish is sent after the apply is done regardless
    // when we call publish, but send it afterwards in case the apply fails
    state.apply(config).await?;
    state.publish_to_mqtt(client, state_topic).await?;

    Ok(())
}

async fn process_state_message(payload: &[u8], state: &mut State, config: &Config) -> anyhow::Result<()> {
    let new_state = serde_json::from_slice::<State>(payload)?;
    info!("Received initial state: {new_state:?}");

    *state = new_state;
    state.apply(config).await?;

    Ok(())
}
