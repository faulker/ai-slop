//! Radio model identity and capabilities.
//!
//! [`Model`] is a plain value that names one supported radio. It travels across
//! the C ABI as its [`Model::id`] string, appears in every JSON envelope, and
//! selects the wire protocol via [`crate::driver::driver_for`].
//!
//! [`Capabilities`] is the machine-readable description of what a model can
//! hold. It lives here, in Rust, rather than in a parallel Swift table so there
//! is exactly one source of truth for "which panes does this radio have" and
//! "what are the legal power levels".

use serde::{Serialize, Serializer};

/// A `(tag, label)` pair for a picker: `tag` is the value that appears in a
/// dump and is accepted by an edit, `label` is what a human reads.
pub type PickerOption = (&'static str, &'static str);

/// The sections an AnyTone codeplug document exposes, in sidebar order. These
/// match the keys of the `anytone` payload in the JSON envelope.
const ANYTONE_SECTIONS: &[&str] = &[
    "channels",
    "zones",
    "scan_lists",
    "contacts",
    "group_lists",
    "radio_ids",
    "aprs",
];

/// Transmit power levels, mirroring [`crate::anytone::codeplug::Power`]'s
/// serialized form.
const ANYTONE_POWER: &[PickerOption] = &[
    ("Low", "Low"),
    ("Mid", "Mid"),
    ("High", "High"),
    ("Turbo", "Turbo"),
];

/// Channel bandwidths, mirroring [`crate::anytone::codeplug::Bandwidth`]'s
/// serialized form.
const ANYTONE_BANDWIDTH: &[PickerOption] = &[("Narrow", "Narrow"), ("Wide", "Wide")];

/// The sections a QYT KT-5000 document exposes, in sidebar order. Prefixed so
/// they can never collide with an AnyTone section of the same name: a QYT
/// channel and an AnyTone channel are different shapes with different field
/// meanings, and the app renders them with different panes.
const QYT_SECTIONS: &[&str] = &["qyt_channels", "qyt_settings", "qyt_bands"];

/// Transmit power levels, mirroring [`crate::qyt::codeplug::Power`]'s
/// serialized form. The field is 2 bits but this radio has only two levels.
const QYT_POWER: &[PickerOption] = &[("High", "High (25 W)"), ("Low", "Low (5 W)")];

/// Channel bandwidths, mirroring [`crate::qyt::codeplug::Bandwidth`]'s
/// serialized form.
const QYT_BANDWIDTH: &[PickerOption] = &[("Wide", "Wide"), ("Narrow", "Narrow")];

/// One supported radio. Adding a variant is the single edit that makes a new
/// model visible to the CLI, the C ABI, and the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// AnyTone AT-D878UVII Plus.
    AnyToneD878UV2,
    /// QYT KT-5000.
    QytKt5000,
}

impl Model {
    /// Every model this build supports, in the order the app should list them.
    pub const ALL: &'static [Model] = &[Model::AnyToneD878UV2, Model::QytKt5000];

    /// The stable wire identifier used in JSON and across the C ABI. Never
    /// change one of these: saved documents and app preferences hold them.
    pub fn id(self) -> &'static str {
        match self {
            Model::AnyToneD878UV2 => "anytone-d878uv2",
            Model::QytKt5000 => "qyt-kt5000",
        }
    }

    /// Parse an [`id`](Model::id) back into a model, or `None` if unknown.
    pub fn from_id(s: &str) -> Option<Model> {
        Model::ALL.iter().copied().find(|m| m.id() == s)
    }

    /// The JSON envelope key holding this model's document payload. Kept short
    /// and separate from [`id`](Model::id) because it reads as a field name.
    pub fn payload_key(self) -> &'static str {
        match self {
            Model::AnyToneD878UV2 => "anytone",
            Model::QytKt5000 => "qyt",
        }
    }

    /// The human-readable model name shown in the UI.
    pub fn display_name(self) -> &'static str {
        match self {
            Model::AnyToneD878UV2 => "AnyTone AT-D878UVII Plus",
            Model::QytKt5000 => "QYT KT-5000",
        }
    }

    /// The exact byte length of a full codeplug image for this model.
    pub fn image_size(self) -> usize {
        match self {
            Model::AnyToneD878UV2 => crate::anytone::device::codeplug_size(),
            Model::QytKt5000 => crate::qyt::device::IMAGE_SIZE,
        }
    }

    /// What this model can hold, for the app's capability-driven UI.
    pub fn capabilities(self) -> Capabilities {
        match self {
            Model::AnyToneD878UV2 => Capabilities {
                sections: ANYTONE_SECTIONS,
                channel_capacity: crate::anytone::codeplug::NUM_CHANNELS,
                channel_name_length: 16,
                supports_dmr: true,
                tone_encoding: "table_index",
                power_options: ANYTONE_POWER,
                bandwidth_options: ANYTONE_BANDWIDTH,
            },
            Model::QytKt5000 => Capabilities {
                sections: QYT_SECTIONS,
                channel_capacity: crate::qyt::codeplug::NUM_CHANNELS,
                channel_name_length: crate::qyt::codeplug::NAME_LEN,
                supports_dmr: false,
                tone_encoding: "hz_tenths",
                power_options: QYT_POWER,
                bandwidth_options: QYT_BANDWIDTH,
            },
        }
    }

    /// Identify which model an image belongs to purely from its bytes.
    ///
    /// The two supported image sizes differ by two orders of magnitude, so
    /// length alone would decide it. The content probe is cheap insurance
    /// against a future model with a colliding size, and against a truncated or
    /// zero-filled file that happens to be the right length.
    pub fn detect(data: &[u8]) -> Option<Model> {
        Model::ALL
            .iter()
            .copied()
            .find(|m| m.image_size() == data.len() && m.content_matches(data))
    }

    /// Whether `data` carries this model's in-image signature. Only the QYT has
    /// one; the AnyTone image has no model tag, so its length is all there is.
    fn content_matches(self, data: &[u8]) -> bool {
        match self {
            Model::AnyToneD878UV2 => true,
            Model::QytKt5000 => {
                let tag = crate::qyt::codeplug::MODEL_TAG;
                data.get(tag..tag + 6) == Some(b"Model:".as_slice())
            }
        }
    }
}

/// What a model can hold. Serialized into every JSON envelope so the app never
/// hardcodes a per-radio table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Document sections this model has, in sidebar order.
    pub sections: &'static [&'static str],
    /// Maximum number of channel slots.
    pub channel_capacity: usize,
    /// Maximum channel name length in characters.
    pub channel_name_length: usize,
    /// Whether the radio has DMR (color code, time slot, contacts, and so on).
    pub supports_dmr: bool,
    /// How CTCSS/DCS tones are encoded in this model's records, so the UI knows
    /// whether a tone field is a table index or a frequency.
    pub tone_encoding: &'static str,
    /// Legal transmit power values.
    #[serde(serialize_with = "serialize_options")]
    pub power_options: &'static [PickerOption],
    /// Legal channel bandwidth values.
    #[serde(serialize_with = "serialize_options")]
    pub bandwidth_options: &'static [PickerOption],
}

/// Serialize picker options as `[{"tag":..,"label":..}]` rather than as bare
/// two-element arrays, so the Swift side decodes them into a named struct.
fn serialize_options<S: Serializer>(
    opts: &&'static [PickerOption],
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(opts.len()))?;
    for (tag, label) in opts.iter() {
        seq.serialize_element(&PickerOptionJson { tag, label })?;
    }
    seq.end()
}

/// Wire shape of one picker option.
#[derive(Serialize)]
struct PickerOptionJson<'a> {
    tag: &'a str,
    label: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_from_id() {
        for m in Model::ALL {
            assert_eq!(Model::from_id(m.id()), Some(*m));
        }
        assert_eq!(Model::from_id("no-such-radio"), None);
    }

    #[test]
    fn detect_model_recognizes_the_anytone_image() {
        let image = vec![0u8; Model::AnyToneD878UV2.image_size()];
        assert_eq!(Model::detect(&image), Some(Model::AnyToneD878UV2));
        assert_eq!(Model::detect(&[0u8; 16]), None);
    }

    #[test]
    fn detect_model_recognizes_the_qyt_image_by_its_tag() {
        let mut image = vec![0xFFu8; Model::QytKt5000.image_size()];
        // Right length, no tag: not enough.
        assert_eq!(Model::detect(&image), None);
        let tag = crate::qyt::codeplug::MODEL_TAG;
        image[tag..tag + 16].copy_from_slice(b"Model:KT-5000   ");
        assert_eq!(Model::detect(&image), Some(Model::QytKt5000));
    }

    #[test]
    fn the_two_models_cannot_be_confused_for_one_another() {
        let anytone = vec![0u8; Model::AnyToneD878UV2.image_size()];
        let mut qyt = vec![0xFFu8; Model::QytKt5000.image_size()];
        let tag = crate::qyt::codeplug::MODEL_TAG;
        qyt[tag..tag + 16].copy_from_slice(b"Model:KT-5000   ");

        assert_eq!(Model::detect(&anytone), Some(Model::AnyToneD878UV2));
        assert_eq!(Model::detect(&qyt), Some(Model::QytKt5000));
        assert_ne!(
            Model::AnyToneD878UV2.image_size(),
            Model::QytKt5000.image_size()
        );
    }

    #[test]
    fn qyt_capabilities_describe_an_analog_only_radio() {
        let caps = Model::QytKt5000.capabilities();
        let v = serde_json::to_value(caps).unwrap();
        assert_eq!(v["sections"][0], "qyt_channels");
        assert_eq!(v["sections"].as_array().unwrap().len(), 3);
        assert_eq!(v["channel_capacity"], 200);
        assert_eq!(v["channel_name_length"], 7);
        assert_eq!(v["supports_dmr"], false);
        assert_eq!(v["tone_encoding"], "hz_tenths");
        assert_eq!(v["power_options"].as_array().unwrap().len(), 2);
        assert_eq!(v["power_options"][0]["tag"], "High");
        assert_eq!(v["bandwidth_options"][0]["tag"], "Wide");
    }

    #[test]
    fn qyt_image_size_is_pinned() {
        assert_eq!(Model::QytKt5000.image_size(), 0x3200);
    }

    #[test]
    fn anytone_image_size_is_pinned() {
        // Guards every later refactor: this number is the app's file-compat check.
        assert_eq!(Model::AnyToneD878UV2.image_size(), 1_789_328);
    }

    #[test]
    fn capabilities_serialize_picker_options_as_objects() {
        let caps = Model::AnyToneD878UV2.capabilities();
        let v = serde_json::to_value(caps).unwrap();
        assert_eq!(v["power_options"][0]["tag"], "Low");
        assert_eq!(v["power_options"][3]["label"], "Turbo");
        assert_eq!(v["bandwidth_options"][1]["tag"], "Wide");
        assert_eq!(v["sections"][0], "channels");
        assert_eq!(v["sections"].as_array().unwrap().len(), 7);
        assert_eq!(v["supports_dmr"], true);
    }
}
