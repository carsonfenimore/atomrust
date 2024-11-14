use paho_mqtt as mqtt;
use serde_json::json;
use serde::Serialize;


pub struct HAMQTTClient {
    topic_prefix: String,
    mqtt: mqtt::AsyncClient,
}

impl HAMQTTClient {
    pub fn new(host: &str, port: u16, username: &str, password: &str) -> Result<Self, Box<dyn std::error::Error>>{
        let topic_prefix = "homeassistant".to_string();
        let mqtt = mqtt::AsyncClient::new(format!("tcp://{}:{}", host, port))?;
        let _ = mqtt.connect(mqtt::ConnectOptionsBuilder::new()
                            .user_name(username)
                            .password(password)
                            .finalize()).wait();
        Ok(Self {
            topic_prefix: topic_prefix,
            mqtt,
        })
    }

    pub fn publish<A: Serialize>(&self, object_id: &str, name: &str, value: A, device_class: &str, unit_of_measurement: &str) 
        -> Result<(), Box<dyn std::error::Error>>{

        let component = "sensor";
        let uniqueid = format!("{}_{}", object_id, name);
        let config_topic = format!("{}/{}/{}/config",self.topic_prefix, component, uniqueid);
        let state_topic = format!("{}/{}/{}/state", self.topic_prefix, component, uniqueid);

        let mut msg_json = json!({
            "name": name,
            "value_template": format!("{{{{ value_json.{} }}}}", name),
            "state_topic": state_topic,
            "device": json!({"identifiers":object_id,"name":object_id}),
            "unique_id": uniqueid,
        });
        if ! device_class.is_empty() {
            msg_json["device_class"] = json!(device_class);
        }
        if ! unit_of_measurement.is_empty() {
            msg_json["unit_of_measurement"] = json!(unit_of_measurement);
        }
        let json_str = msg_json.to_string();
        let msg = mqtt::Message::new(config_topic, json_str, mqtt::QOS_1);
        let _ = self.mqtt.publish(msg).wait();
 
        let json_state = json!({
            name: value });
        let json_state_str = json_state.to_string();
        let state_msg = mqtt::Message::new(state_topic, json_state_str, mqtt::QOS_1);
        let _ = self.mqtt.publish(state_msg).wait();
        Ok(())
  }


}

