//! Shared upstream message types used across multiple stream
//! clients. Currently scoped to AIS — OpenSky / OREF / Telegram
//! land their own typed shapes alongside their clients.
//!
//! The aisstream.io v0 wire shape is documented at
//! <https://aisstream.io/documentation>. We deserialise the
//! envelope strictly (`MessageType`, `MetaData`, `Message`) and
//! preserve `Message` as a raw `serde_json::Value` so panel-side
//! consumers can pluck the fields they need without us having to
//! mirror every concrete AIS message type (PositionReport, ShipStaticData,
//! ExtendedClassBPositionReport, …).

use serde::{Deserialize, Serialize};

/// Top-level aisstream.io v0 envelope. Every push from the
/// upstream WebSocket conforms to this shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AisEnvelope {
    /// One of: `PositionReport`, `ShipStaticData`,
    /// `ExtendedClassBPositionReport`, `StaticDataReport`,
    /// `AddressedSafetyMessage`, `BinaryBroadcastMessage`,
    /// `BinaryAddressedMessage`, … See aisstream docs for the
    /// full list.
    #[serde(rename = "MessageType")]
    pub message_type: String,

    /// Per-message metadata: `MMSI`, `ShipName`, `latitude`,
    /// `longitude`, `time_utc`. We keep this as a structured
    /// type since every consumer reads it.
    #[serde(rename = "MetaData")]
    pub metadata: AisMetadata,

    /// Concrete message body — kept as raw JSON so consumers can
    /// pluck the fields they need without us mirroring every
    /// concrete AIS message type.
    #[serde(rename = "Message")]
    pub message: serde_json::Value,
}

/// `MetaData` block. Fields are optional because the upstream
/// occasionally drops `ShipName` / `time_utc` for partial
/// updates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AisMetadata {
    /// Maritime Mobile Service Identity — primary vessel id.
    #[serde(rename = "MMSI")]
    pub mmsi: u32,

    /// Vessel name (whitespace-trimmed by upstream).
    #[serde(rename = "ShipName", default)]
    pub ship_name: Option<String>,

    /// Latest known latitude in decimal degrees.
    #[serde(default)]
    pub latitude: f64,

    /// Latest known longitude in decimal degrees.
    #[serde(default)]
    pub longitude: f64,

    /// ISO 8601 timestamp of the upstream observation.
    #[serde(rename = "time_utc", default)]
    pub time_utc: Option<String>,
}

/// Manual `Eq` is unsafe on f64 due to NaN — we deliberately
/// derive `PartialEq` only and document why callers should not
/// rely on `Eq`.
impl Eq for AisMetadata {}

/// Subscribe-message shape sent on connect. aisstream.io requires
/// the API key + at least one bounding box.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AisSubscribe {
    /// Per-tenant API key.
    #[serde(rename = "APIKey")]
    pub api_key: String,

    /// Bounding boxes as `[[[lat1, lon1], [lat2, lon2]], ...]`.
    /// World box is `[[[-90.0, -180.0], [90.0, 180.0]]]`.
    #[serde(rename = "BoundingBoxes")]
    pub bounding_boxes: Vec<Vec<[f64; 2]>>,

    /// Optional message-type filter. When `None`, every type is
    /// emitted; the relay subscribes to every type and filters
    /// downstream.
    #[serde(
        rename = "FilterMessageTypes",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub filter_message_types: Option<Vec<String>>,
}

impl AisSubscribe {
    /// Convenience: subscribe to the entire planet.
    #[must_use]
    pub fn world(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            bounding_boxes: vec![vec![[-90.0, -180.0], [90.0, 180.0]]],
            filter_message_types: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_envelope_json() -> &'static str {
        r#"{
            "MessageType": "PositionReport",
            "MetaData": {
                "MMSI": 367123456,
                "ShipName": "EVER GIVEN          ",
                "latitude": 30.5234,
                "longitude": 32.3456,
                "time_utc": "2026-05-02T12:34:56.000+0000"
            },
            "Message": {
                "PositionReport": {
                    "Sog": 12.4,
                    "Cog": 245.0,
                    "Heading": 244,
                    "NavigationalStatus": 0
                }
            }
        }"#
    }

    #[test]
    fn parses_position_report_envelope() {
        let env: AisEnvelope = serde_json::from_str(sample_envelope_json()).unwrap();
        assert_eq!(env.message_type, "PositionReport");
        assert_eq!(env.metadata.mmsi, 367123456);
        assert_eq!(
            env.metadata.ship_name.as_deref().unwrap().trim(),
            "EVER GIVEN"
        );
        assert!((env.metadata.latitude - 30.5234).abs() < 1e-9);
        assert!((env.metadata.longitude - 32.3456).abs() < 1e-9);
        let pr = env.message.pointer("/PositionReport/Sog").unwrap();
        assert!((pr.as_f64().unwrap() - 12.4).abs() < 1e-9);
    }

    #[test]
    fn metadata_with_missing_optional_fields_parses() {
        let json = r#"{
            "MessageType": "ShipStaticData",
            "MetaData": { "MMSI": 367000001, "latitude": 0.0, "longitude": 0.0 },
            "Message": { "ShipStaticData": {} }
        }"#;
        let env: AisEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.metadata.ship_name, None);
        assert_eq!(env.metadata.time_utc, None);
    }

    #[test]
    fn subscribe_world_serialises_with_full_box() {
        let sub = AisSubscribe::world("test-key");
        let json = serde_json::to_value(&sub).unwrap();
        assert_eq!(json["APIKey"], "test-key");
        let boxes = &json["BoundingBoxes"];
        assert_eq!(boxes[0][0], serde_json::json!([-90.0, -180.0]));
        assert_eq!(boxes[0][1], serde_json::json!([90.0, 180.0]));
        assert!(json.get("FilterMessageTypes").is_none());
    }

    #[test]
    fn subscribe_with_filter_serialises_filter_field() {
        let sub = AisSubscribe {
            api_key: "k".into(),
            bounding_boxes: vec![vec![[-1.0, -1.0], [1.0, 1.0]]],
            filter_message_types: Some(vec!["PositionReport".into()]),
        };
        let json = serde_json::to_value(&sub).unwrap();
        assert_eq!(json["FilterMessageTypes"][0], "PositionReport");
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let env: AisEnvelope = serde_json::from_str(sample_envelope_json()).unwrap();
        let s = serde_json::to_string(&env).unwrap();
        let back: AisEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back, env);
    }
}
