use std::collections::HashMap;

use serde::{Deserialize, Serialize};

macro_rules! public_struct {
    (
        $(#[$meta:meta])*
        struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $fname:ident : $fty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $(
                $(#[$field_meta])*
                pub $fname: $fty
            ),*
        }
    };
}

public_struct! {
#[derive(Debug, Serialize, Deserialize)]
struct GrainRate {
    numerator: i32,
    denominator: i32,
}
}

public_struct! {
#[derive(Debug, Serialize, Deserialize)]
struct Component {
    name: String,
    width: i32,
    height: i32,
    bit_depth: u8,
}
}

public_struct! {
    #[derive(Debug, Serialize, Deserialize)]
    struct FlowDefVideo {
        #[serde(default, rename = "$copyright")]
        copyright: String,
        #[serde(default, rename = "$license")]
        license: String,

        description: String,
        id: String,
        tags: HashMap<String, Vec<String>>,
        format: String,
        label: String,
        parents: Vec<String>,
        media_type: String,
        grain_rate: GrainRate,
        frame_width: i32,
        frame_height: i32,
        interlace_mode: String,
        colorspace: String,
        components: Vec<Component>,
    }
}
public_struct! {
#[derive(Debug, Serialize, Deserialize)]
struct SampleRate {
    numerator: i32,
}
}

public_struct! {
#[derive(Debug, Serialize, Deserialize)]
struct FlowDefAudio {
    #[serde(rename = "$copyright")]
    copyright: String,
    #[serde(rename = "$license")]
    license: String,
    description: String,
    format: String,
    tags: HashMap<String, Vec<String>>,
    label: String,
    id: String,
    media_type: String,
    sample_rate: SampleRate,
    channel_count: i32,
    bit_depth: u8,
    parents: Vec<String>,
}
}

public_struct! {
struct FlowDef {
    video: Option<FlowDefVideo>,
    audio: Option<FlowDefAudio>,
}
}
