use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use paho_mqtt as mqtt;
use serde_json::json;
use serde::Serialize;

/// Sensors that stop reporting go "unavailable" in Home Assistant after this long.
const EXPIRE_AFTER_SECS: u64 = 120;

pub struct HAMQTTClient {
    topic_prefix: String,
    mqtt: mqtt::AsyncClient,
    /// Sensors whose discovery config has been sent on the current connection.
    announced: Arc<Mutex<HashSet<String>>>,
}

impl HAMQTTClient {
    /// Creates the client and starts connecting in the background. Never blocks
    /// on the broker: if it is down at boot we keep retrying, and after the first
    /// connection paho reconnects automatically.
    pub fn new(host: &str, port: u16, username: &str, password: &str) -> Result<Self, Box<dyn std::error::Error>>{
        let topic_prefix = "homeassistant".to_string();
        let mqtt = mqtt::AsyncClient::new(format!("tcp://{}:{}", host, port))?;
        let announced: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        // Re-announce discovery configs after every (re)connect, in case the
        // broker lost its retained messages.
        mqtt.set_connected_callback({
            let announced = announced.clone();
            move |_| {
                tracing::info!("MQTT connected");
                if let Ok(mut announced) = announced.lock() {
                    announced.clear();
                }
            }
        });
        mqtt.set_connection_lost_callback(|_| tracing::warn!("MQTT connection lost; reconnecting"));

        let opts = mqtt::ConnectOptionsBuilder::new()
            .user_name(username)
            .password(password)
            .keep_alive_interval(Duration::from_secs(30))
            .clean_session(true)
            .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(60))
            .finalize();

        let cli = mqtt.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                match cli.connect(opts.clone()).await {
                    Ok(_) => break,
                    Err(err) => {
                        tracing::warn!(%err, "MQTT connect failed; retrying in {:?}", backoff);
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                    }
                }
            }
        });

        Ok(Self {
            topic_prefix,
            mqtt,
            announced,
        })
    }

    pub async fn publish<A: Serialize>(&self, object_id: &str, name: &str, value: A, device_class: &str, unit_of_measurement: &str)
        -> Result<(), Box<dyn std::error::Error>>{
        if !self.mqtt.is_connected() {
            return Ok(());
        }

        let component = "sensor";
        let uniqueid = format!("{}_{}", object_id, name);
        let state_topic = format!("{}/{}/{}/state", self.topic_prefix, component, uniqueid);

        let needs_announce = self
            .announced
            .lock()
            .map(|mut announced| announced.insert(uniqueid.clone()))
            .unwrap_or(true);
        if needs_announce {
            let config_topic = format!("{}/{}/{}/config",self.topic_prefix, component, uniqueid);
            let mut msg_json = json!({
                "name": name,
                "value_template": format!("{{{{ value_json.{} }}}}", name),
                "state_topic": state_topic,
                "device": json!({"identifiers":object_id,"name":object_id}),
                "unique_id": uniqueid,
                "expire_after": EXPIRE_AFTER_SECS,
            });
            if ! device_class.is_empty() {
                msg_json["device_class"] = json!(device_class);
            }
            if ! unit_of_measurement.is_empty() {
                msg_json["unit_of_measurement"] = json!(unit_of_measurement);
            }
            let msg = mqtt::Message::new_retained(config_topic, msg_json.to_string(), mqtt::QOS_1);
            if let Err(err) = self.mqtt.publish(msg).await {
                // Try again on the next sample.
                if let Ok(mut announced) = self.announced.lock() {
                    announced.remove(&uniqueid);
                }
                return Err(err.into());
            }
        }

        // State updates are periodic, so fire-and-forget at QoS 0: never wait
        // on the broker from the publishing loops.
        let json_state = json!({ name: value });
        let state_msg = mqtt::Message::new(state_topic, json_state.to_string(), mqtt::QOS_0);
        let _ = self.mqtt.publish(state_msg);
        Ok(())
    }
}
