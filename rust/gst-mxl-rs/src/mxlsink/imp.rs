// Copyright (C) 2018 Sebastian Dröge <sebastian@centricular.com>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;
use gst_base::subclass::prelude::*;

use mxl::config::get_mxl_so_path;
use mxl::FlowInfo;
use mxl::MxlInstance;

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;

use serde::Serialize;

use crate::mxlsink;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "rssink",
        gst::DebugColorFlags::empty(),
        Some("Rust MXL Sink"),
    )
});

const DEFAULT_FLOW_ID: &str = "";
const DEFAULT_DOMAIN: &str = "";

#[derive(Debug, Serialize)]
struct GrainRate {
    numerator: i32,
    denominator: i32,
}

#[derive(Debug, Serialize)]
struct Component {
    name: String,
    width: i32,
    height: i32,
    bit_depth: u8,
}

#[derive(Debug, Serialize)]
struct FlowDef {
    #[serde(rename = "$copyright")]
    copyright: String,
    #[serde(rename = "$license")]
    license: String,

    description: String,
    id: String,
    tags: HashMap<String, String>,
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

#[derive(Debug, Clone)]
struct Settings {
    flow_id: String,
    domain: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            flow_id: DEFAULT_FLOW_ID.to_owned(),
            domain: DEFAULT_DOMAIN.to_owned(),
        }
    }
}

#[derive(Default)]
struct State {
    pub instance: Option<MxlInstance>,
    pub flow: Option<FlowInfo>,
}

struct ClockWait {
    clock_id: Option<gst::SingleShotClockId>,
    flushing: bool,
}

impl Default for ClockWait {
    fn default() -> ClockWait {
        ClockWait {
            clock_id: None,
            flushing: true,
        }
    }
}

#[derive(Default)]
pub struct MxlSink {
    settings: Mutex<Settings>,
    state: Mutex<State>,
    clock_wait: Mutex<ClockWait>,
}

#[glib::object_subclass]
impl ObjectSubclass for MxlSink {
    const NAME: &'static str = "GstRsMxlSink";
    type Type = mxlsink::MxlSink;
    type ParentType = gst_base::BaseSink;
}

impl ObjectImpl for MxlSink {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
            vec![
                glib::ParamSpecString::builder("flow-id")
                    .nick("FlowID")
                    .blurb("Flow ID")
                    .default_value(DEFAULT_FLOW_ID)
                    .mutable_ready()
                    .build(),
                glib::ParamSpecString::builder("domain")
                    .nick("Domain")
                    .blurb("Domain")
                    .default_value(DEFAULT_DOMAIN)
                    .mutable_ready()
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn constructed(&self) {
        self.parent_constructed();
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        if let Ok(mut settings) = self.settings.lock() {
            match pspec.name() {
                "flow-id" => {
                    if let Ok(flow_id) = value.get::<String>() {
                        gst::info!(
                            CAT,
                            imp = self,
                            "Changing flow-id from {} to {}",
                            settings.flow_id,
                            flow_id
                        );
                        settings.flow_id = flow_id;
                    } else {
                        gst::error!(CAT, imp = self, "Invalid type for flow-id property");
                    }
                }
                "domain" => {
                    if let Ok(domain) = value.get::<String>() {
                        gst::info!(
                            CAT,
                            imp = self,
                            "Changing domain from {} to {}",
                            settings.domain,
                            domain
                        );
                        settings.domain = domain;
                    } else {
                        gst::error!(CAT, imp = self, "Invalid type for domain property");
                    }
                }
                other => {
                    gst::error!(CAT, imp = self, "Unknown property '{}'", other);
                }
            }
        } else {
            gst::error!(
                CAT,
                imp = self,
                "Settings mutex poisoned — property change ignored"
            );
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        if let Ok(settings) = self.settings.lock() {
            match pspec.name() {
                "flow-id" => settings.flow_id.to_value(),
                "domain" => settings.domain.to_value(),
                _ => {
                    gst::error!(CAT, imp = self, "Unknown property {}", pspec.name());
                    glib::Value::from(&"")
                }
            }
        } else {
            gst::error!(CAT, imp = self, "Settings mutex poisoned");
            glib::Value::from(&"")
        }
    }
}

impl GstObjectImpl for MxlSink {}

impl ElementImpl for MxlSink {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Gstreamer MXL Sink",
                "Sink/Video",
                "Generates video flow",
                "Bisect",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        use std::sync::LazyLock;

        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst::Caps::new_any();

            let sink_pad_template = gst::PadTemplate::new(
                "sink",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &caps,
            )
            .expect("Failed to create sink pad template");

            vec![sink_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        self.parent_change_state(transition)
    }
}

impl BaseSinkImpl for MxlSink {
    fn start(&self) -> Result<(), gst::ErrorMessage> {
        let mut state = self.state.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
        })?;
        *state = Default::default();
        self.unlock_stop()?;
        let settings = self.settings.lock().map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to get settings mutex: {}", e]
            )
        })?;
        state.instance = Some(init_mxl_instance(&settings)?);
        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        let state = self.state.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
        })?;
        self.unlock()?;
        let settings = self.settings.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
        })?;
        state
            .instance
            .as_ref()
            .ok_or(gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to get instance on exit"]
            ))?
            .destroy_flow(&settings.flow_id)
            .map_err(|e| {
                gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
            })?;

        gst::info!(CAT, imp = self, "Stopped");
        Ok(())
    }

    fn render(&self, buffer: &gst::Buffer) -> Result<gst::FlowSuccess, gst::FlowError> {
        self.parent_render(buffer)
    }

    fn prepare(&self, buffer: &gst::Buffer) -> Result<gst::FlowSuccess, gst::FlowError> {
        self.parent_prepare(buffer)
    }

    fn render_list(&self, list: &gst::BufferList) -> Result<gst::FlowSuccess, gst::FlowError> {
        self.parent_render_list(list)
    }

    fn prepare_list(&self, list: &gst::BufferList) -> Result<gst::FlowSuccess, gst::FlowError> {
        self.parent_prepare_list(list)
    }

    fn query(&self, query: &mut gst::QueryRef) -> bool {
        BaseSinkImplExt::parent_query(self, query)
    }

    fn event(&self, event: gst::Event) -> bool {
        self.parent_event(event)
    }

    fn caps(&self, filter: Option<&gst::Caps>) -> Option<gst::Caps> {
        self.parent_caps(filter)
    }

    fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock state mutex: {}", e))?;

        let settings = self
            .settings
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock settings mutex: {}", e))?;

        let structure = caps
            .structure(0)
            .ok_or_else(|| gst::loggable_error!(CAT, "No structure in caps {}", caps))?;

        let format = structure
            .get::<String>("format")
            .unwrap_or_else(|_| "v210".to_string());
        let width = structure.get::<i32>("width").unwrap_or(1920);
        let height = structure.get::<i32>("height").unwrap_or(1080);
        let framerate = structure
            .get::<gst::Fraction>("framerate")
            .unwrap_or_else(|_| gst::Fraction::new(30000, 1001));
        let interlace_mode = structure
            .get::<String>("interlace-mode")
            .unwrap_or_else(|_| "progressive".to_string());
        let colorimetry = structure
            .get::<String>("colorimetry")
            .unwrap_or_else(|_| "BT709".to_string());
        let flow_id = &settings.flow_id;
        let flow_def = FlowDef {
            copyright:
                "SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project."
                    .into(),
            license: "SPDX-License-Identifier: Apache-2.0".into(),
            description: format!(
                "MXL Test Flow, 1080p{}",
                framerate.numer() / framerate.denom()
            )
            .into(),
            id: flow_id.deref().into(),
            tags: HashMap::new(),
            format: "urn:x-nmos:format:video".into(),
            label: format!(
                "MXL Test Flow, 1080p{}",
                framerate.numer() / framerate.denom()
            )
            .into(),
            parents: vec![],
            media_type: format!("video/{}", format).into(),
            grain_rate: GrainRate {
                numerator: framerate.numer(),
                denominator: framerate.denom(),
            },
            frame_width: width,
            frame_height: height,
            interlace_mode: interlace_mode,
            colorspace: colorimetry,
            components: vec![
                Component {
                    name: "Y".into(),
                    width: width,
                    height: height,
                    bit_depth: 10,
                },
                Component {
                    name: "Cb".into(),
                    width: width / 2,
                    height: height,
                    bit_depth: 10,
                },
                Component {
                    name: "Cr".into(),
                    width: width / 2,
                    height: height,
                    bit_depth: 10,
                },
            ],
        };
        let flow = state
            .instance
            .as_ref()
            .ok_or(gst::loggable_error!(CAT, "Failed to get instance: is None"))?
            .create_flow(
                serde_json::to_string(&flow_def)
                    .map_err(|e| gst::loggable_error!(CAT, "Failed to convert: {}", e))?
                    .as_str(),
                None,
            )
            .map_err(|e| gst::loggable_error!(CAT, "Failed to create flow: {}", e))?;
        state.flow = Some(flow);
        Ok(())
    }

    fn fixate(&self, caps: gst::Caps) -> gst::Caps {
        self.parent_fixate(caps)
    }

    fn unlock(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp = self, "Unlocking");
        let mut clock_wait = self.clock_wait.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to lock clock: {}", e])
        })?;
        if let Some(clock_id) = clock_wait.clock_id.take() {
            clock_id.unschedule();
        }
        clock_wait.flushing = true;

        Ok(())
    }

    fn unlock_stop(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp = self, "Unlock stop");
        let mut clock_wait = self.clock_wait.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to lock clock: {}", e])
        })?;
        clock_wait.flushing = false;

        Ok(())
    }

    fn propose_allocation(
        &self,
        query: &mut gst::query::Allocation,
    ) -> Result<(), gst::LoggableError> {
        self.parent_propose_allocation(query)
    }
}

fn init_mxl_instance(
    settings: &MutexGuard<'_, Settings>,
) -> Result<MxlInstance, gst::ErrorMessage> {
    let mxl_api = mxl::load_api(get_mxl_so_path())
        .map_err(|e| gst::error_msg!(gst::CoreError::Failed, ["Failed to load MXL API: {}", e]))?;

    let mxl_instance =
        mxl::MxlInstance::new(mxl_api, settings.domain.as_str(), "").map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to load MXL instance: {}", e]
            )
        })?;

    Ok(mxl_instance)
}

#[cfg(test)]
mod tests {
    use gst::{CoreError, Fraction};

    use super::*;

    #[test]
    fn flow_def_generation() {
        let flow_id = String::from("5fbec3b1-1b0f-417d-9059-8b94a47197ed");
        let width = 1920;
        let height = 1080;
        let framerate = Fraction::new(30000, 1001);
        let interlace_mode = "progressive".to_string();
        let colorimetry = "BT709".to_string();
        let format = "v210".to_string();

        let flow_def = FlowDef {
            copyright:
                "SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project."
                    .into(),
            license: "SPDX-License-Identifier: Apache-2.0".into(),
            description: format!(
                "MXL Test Flow, 1080p{}",
                framerate.numer() / framerate.denom()
            )
            .into(),
            id: flow_id.to_string(),
            tags: HashMap::new(),
            format: "urn:x-nmos:format:video".into(),
            label: format!(
                "MXL Test Flow, 1080p{}",
                framerate.numer() / framerate.denom()
            )
            .into(),
            parents: vec![],
            media_type: format!("video/{}", format),
            grain_rate: GrainRate {
                numerator: framerate.numer(),
                denominator: framerate.denom(),
            },
            frame_width: width,
            frame_height: height,
            interlace_mode,
            colorspace: colorimetry,
            components: vec![
                Component {
                    name: "Y".into(),
                    width,
                    height,
                    bit_depth: 10,
                },
                Component {
                    name: "Cb".into(),
                    width: width / 2,
                    height,
                    bit_depth: 10,
                },
                Component {
                    name: "Cr".into(),
                    width: width / 2,
                    height,
                    bit_depth: 10,
                },
            ],
        };

        let json = serde_json::to_value(&flow_def).unwrap();

        let expected = serde_json::json!({
            "$copyright": "SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project.",
            "$license": "SPDX-License-Identifier: Apache-2.0",
            "description": "MXL Test Flow, 1080p29",
            "id": "5fbec3b1-1b0f-417d-9059-8b94a47197ed",
            "tags": {},
            "format": "urn:x-nmos:format:video",
            "label": "MXL Test Flow, 1080p29",
            "parents": [],
            "media_type": "video/v210",
            "grain_rate": {
                "numerator": 30000,
                "denominator": 1001
            },
            "frame_width": 1920,
            "frame_height": 1080,
            "interlace_mode": "progressive",
            "colorspace": "BT709",
            "components": [
                {
                    "name": "Y",
                    "width": 1920,
                    "height": 1080,
                    "bit_depth": 10
                },
                {
                    "name": "Cb",
                    "width": 960,
                    "height": 1080,
                    "bit_depth": 10
                },
                {
                    "name": "Cr",
                    "width": 960,
                    "height": 1080,
                    "bit_depth": 10
                }
            ]
        });
        println!("{:#?}", json);
        assert_eq!(json, expected);
    }
    #[test]
    fn set_caps() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsink", gst::Rank::NONE, MxlSink::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let sink = gst::ElementFactory::make("mxlsink")
            .property("flow-id", "7fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .expect("Failed to create element");
        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many(&[&sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.to_string()))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.to_string()))?;
        let sink_pad = sink
            .static_pad("sink")
            .ok_or(CoreError::Failed)
            .map_err(|_| glib::Error::new(CoreError::Pad, "Sink pad failed"))?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "v210")
            .field("width", 1920)
            .field("height", 1080)
            .field("framerate", gst::Fraction::new(30000, 1001))
            .build();

        sink_pad.send_event(gst::event::Caps::new(&caps));
        if let Some(caps) = sink_pad.current_caps() {
            println!("Negotiated caps: {}", caps.to_string());
        } else {
            println!("No negotiated caps found");
        }
        // let caps = gst::Caps::builder("video/x-raw")
        //     .field("format", "v210")
        //     .field("width", 1920)
        //     .field("height", 1080)
        //     .field("framerate", gst::Fraction::new(30000, 1001))
        //     .field("interlace-mode", "progressive")
        //     .field("colorimetry", "BT709")
        //     .build();

        // let sinkpad = sink.static_pad("sink").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        pipeline
            .set_state(gst::State::Null)
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.to_string()))?;
        Ok(())
    }
}
