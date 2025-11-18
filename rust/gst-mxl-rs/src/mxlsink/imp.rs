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
use gst::ClockTime;
use gst_base::prelude::BaseSinkExt;
use gst_base::subclass::prelude::*;

use mxl::config::get_mxl_so_path;
use mxl::FlowInfo;
use mxl::GrainWriter;
use mxl::MxlInstance;
use mxl::Rational;
use mxl::SamplesWriter;
use tracing::trace;

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Instant;

use crate::flowdef::*;
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

struct State {
    pub instance: MxlInstance,
    pub flow: Option<FlowInfo>,
    pub video: Option<VideoState>,
    pub audio: Option<AudioState>,
    pub initial_time: Option<InitialTime>,
}

struct VideoState {
    pub writer: GrainWriter,
    pub grain_index: u64,
    pub grain_rate: Rational,
    pub grain_count: u32,
}

struct AudioState {
    pub writer: SamplesWriter,
    pub bit_depth: u8,
    pub batch_size: usize,
    pub flow_def: FlowDefAudio,
}

#[derive(Default)]
struct Context {
    pub state: Option<State>,
}

#[derive(Default, Debug, Clone)]
struct InitialTime {
    index: u64,
    gst_time: gst::ClockTime,
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
    context: Mutex<Context>,
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
        #[cfg(feature = "tracing")]
        {
            use tracing_subscriber::filter::LevelFilter;
            use tracing_subscriber::util::SubscriberInitExt;
            let result = tracing_subscriber::fmt()
                .compact()
                .with_file(true)
                .with_line_number(true)
                .with_thread_ids(true)
                .with_target(false)
                .with_max_level(LevelFilter::TRACE)
                .with_ansi(true)
                .finish()
                .try_init();
        }
        self.parent_constructed();
        self.obj().set_sync(true);
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
            let mut caps = gst::Caps::new_empty();
            {
                let caps_mut = caps.make_mut();

                caps_mut.append(
                    gst::Caps::builder("video/x-raw")
                        .field("format", "v210")
                        .build(),
                );
                for ch in 1..64 {
                    let mask = gst::Bitmask::from((1u64 << ch) - 1);
                    caps.make_mut().append(
                        gst::Caps::builder("audio/x-raw")
                            .field("format", "F32LE")
                            .field("layout", "interleaved")
                            .field("channels", ch)
                            .field("channel-mask", mask)
                            .build(),
                    );
                }
            }

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
        let mut context = self.context.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
        })?;
        self.unlock_stop()?;
        let settings = self.settings.lock().map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to get settings mutex: {}", e]
            )
        })?;
        let instance = init_mxl_instance(&settings)?;
        context.state = Some(State {
            instance: instance,
            flow: None,
            initial_time: None,
            video: None,
            audio: None,
        });

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        let context = self.context.lock().map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to get context mutex: {}", e]
            )
        })?;
        self.unlock()?;
        let state = context.state.as_ref().ok_or(gst::error_msg!(
            gst::CoreError::Failed,
            ["Failed to get state"]
        ))?;
        let settings = self.settings.lock().map_err(|e| {
            gst::error_msg!(gst::CoreError::Failed, ["Failed to get state mutex: {}", e])
        })?;
        if state.flow.is_some() {
            if let Err(e) = state.instance.destroy_flow(&settings.flow_id) {
                gst::warning!(CAT, imp = self, "Failed to destroy flow: {}", e);
            }
        }

        gst::info!(CAT, imp = self, "Stopped");
        Ok(())
    }

    fn render(&self, buffer: &gst::Buffer) -> Result<gst::FlowSuccess, gst::FlowError> {
        trace!("START RENDER");

        let mut context = self.context.lock().map_err(|_| gst::FlowError::Error)?;
        let state = context.state.as_mut().ok_or(gst::FlowError::Error)?;
        if state.video.is_some() {
            render_video(self, state, buffer)
        } else {
            render_audio(self, state, buffer)
        }
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
        let mut context = self
            .context
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock context mutex: {}", e))?;
        let state = context
            .state
            .as_mut()
            .ok_or(gst::loggable_error!(CAT, "Failed to get state",))?;

        let settings = self
            .settings
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock settings mutex: {}", e))?;

        let structure = caps
            .structure(0)
            .ok_or_else(|| gst::loggable_error!(CAT, "No structure in caps {}", caps))?;
        let name = structure.name();
        if name == "audio/x-raw" {
            let info = gst_audio::AudioInfo::from_caps(caps)
                .map_err(|e| gst::loggable_error!(CAT, "Invalid audio caps: {}", e))?;

            let channels = info.channels() as i32;
            let rate = info.rate() as i32;
            let bit_depth = info.depth() as u8;
            let format = info.format().to_string();
            let flow_id = &settings.flow_id;

            let flow_def = FlowDefAudio {
                copyright:
                    "SPDX-FileCopyrightText: 2025 Contributors to the Media eXchange Layer project."
                        .into(),
                license: "SPDX-License-Identifier: Apache-2.0".into(),
                description: "MXL Audio Flow".into(),
                format: "urn:x-nmos:format:audio".into(),
                tags: [].into(),
                label: "MXL Audio Flow".into(),
                id: flow_id.deref().into(),
                media_type: format!("audio/float32" /*format*/,),
                sample_rate: SampleRate { numerator: rate },
                channel_count: channels,
                bit_depth: bit_depth as u8,
                parents: vec![],
            };

            let instance = &state.instance;
            let flow = instance
                .create_flow(
                    serde_json::to_string(&flow_def)
                        .map_err(|e| gst::loggable_error!(CAT, "Failed to convert: {}", e))?
                        .as_str(),
                    None,
                )
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create audio flow: {}", e))?;

            let writer = instance
                .create_flow_writer(flow_id.as_str())
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create flow writer: {}", e))?
                .to_samples_writer()
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create grain writer: {}", e))?;
            state.audio = Some(AudioState {
                writer: writer,
                bit_depth,
                batch_size: (rate as usize / 100),
                flow_def,
            });
            state.flow = Some(flow);

            trace!(
                "Made it to the end of set_caps with format {}, channel_count {}, sample_rate {}, bit_depth {}",
                format,
                channels,
                rate,
                bit_depth
            );
            return Ok(());
        } else {
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
            let flow_def = FlowDefVideo {
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
            let instance = &state.instance;
            let flow = instance
                .create_flow(
                    serde_json::to_string(&flow_def)
                        .map_err(|e| gst::loggable_error!(CAT, "Failed to convert: {}", e))?
                        .as_str(),
                    None,
                )
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create flow: {}", e))?;
            let grain_rate = flow
                .discrete_flow_info()
                .map_err(|e| gst::loggable_error!(CAT, "Failed to get grain rate: {}", e))?
                .grainRate;
            let grain_count = flow
                .discrete_flow_info()
                .map_err(|e| gst::loggable_error!(CAT, "Failed to get grain count: {}", e))?
                .grainCount;
            let writer = instance
                .create_flow_writer(flow_id.as_str())
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create flow writer: {}", e))?
                .to_grain_writer()
                .map_err(|e| gst::loggable_error!(CAT, "Failed to create grain writer: {}", e))?;
            let rate = flow
                .discrete_flow_info()
                .map_err(|_| gst::loggable_error!(CAT, "Failed to get instance: is None"))?
                .grainRate;
            let index = instance.get_current_index(&rate);
            state.video = Some(VideoState {
                writer: writer,
                grain_index: index,
                grain_rate,
                grain_count,
            });
            state.flow = Some(flow);

            Ok(())
        }
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

fn render_video(
    mxlsink: &mxlsink::imp::MxlSink,
    state: &mut mxlsink::imp::State,
    buffer: &gst::Buffer,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let current_index = state.instance.get_current_index(
        &state
            .flow
            .as_ref()
            .ok_or(gst::FlowError::Error)?
            .discrete_flow_info()
            .map_err(|_| gst::FlowError::Error)?
            .grainRate,
    );
    let video_state = state.video.as_mut().ok_or(gst::FlowError::Error)?;
    let gst_time = mxlsink
        .obj()
        .current_running_time()
        .ok_or(gst::FlowError::Error)?;
    let _ = state.initial_time.get_or_insert_with(|| InitialTime {
        index: current_index,
        gst_time: gst_time,
    });
    let initial_info = state.initial_time.as_ref().ok_or(gst::FlowError::Error)?;
    let mut index = current_index;
    match buffer.pts() {
        Some(pts) => {
            let pts = pts + initial_info.gst_time;
            index = state
                .instance
                .timestamp_to_index(pts.nseconds(), &video_state.grain_rate)
                .map_err(|_| gst::FlowError::Error)?
                + initial_info.index;

            trace!(
                    "PTS {:?} mapped to grain index {}, current index is {} and running time is {} delta= {}",
                    pts,
                    index,
                    current_index,
                    gst_time,
                    if pts > gst_time {pts - gst_time} else {ClockTime::from_mseconds(0)}
                );
            if index > current_index {
                if index - current_index > video_state.grain_count as u64 {
                    index = current_index + video_state.grain_count as u64 - 1;
                }
            }
            video_state.grain_index = index;
        }
        None => {
            video_state.grain_index = current_index;
        }
    }

    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
    let data = map.as_slice();

    let mut access = video_state
        .writer
        .open_grain(index)
        .map_err(|_| gst::FlowError::Error)?;

    let payload = access.payload_mut();
    let copy_len = std::cmp::min(payload.len(), data.len());

    let commit_time = Instant::now();
    payload[..copy_len].copy_from_slice(&data[..copy_len]);
    access
        .commit(copy_len as u32)
        .map_err(|_| gst::FlowError::Error)?;
    trace!(
        "Commit time: {}us of grain: {}",
        commit_time.elapsed().as_micros(),
        index
    );
    video_state.grain_index += 1;
    trace!("END RENDER");
    Ok(gst::FlowSuccess::Ok)
}
fn render_audio(
    mxlsink: &mxlsink::imp::MxlSink,
    state: &mut mxlsink::imp::State,
    buffer: &gst::Buffer,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
    let src = map.as_slice();
    let audio_state = state.audio.as_mut().ok_or(gst::FlowError::Error)?;

    let bytes_per_sample = (audio_state.flow_def.bit_depth / 8) as usize;
    trace!(
        "received buffer size: {}, channel count: {}, bit-depth: {}, bytes-per-sample: {}",
        src.len(),
        audio_state.flow_def.channel_count,
        audio_state.bit_depth,
        bytes_per_sample
    );

    let samples_per_buffer =
        src.len() / (audio_state.flow_def.channel_count as usize * bytes_per_sample);
    audio_state.batch_size = samples_per_buffer;

    let flow = state.flow.as_ref().ok_or(gst::FlowError::Error)?;
    let flow_info = flow
        .continuous_flow_info()
        .map_err(|_| gst::FlowError::Error)?;
    let sample_rate = flow_info.sampleRate;
    let buffer_length = flow_info.bufferLength as u64;
    let current_index = state.instance.get_current_index(&sample_rate);
    let gst_time = mxlsink
        .obj()
        .current_running_time()
        .ok_or(gst::FlowError::Error)?;

    let _ = state
        .initial_time
        .get_or_insert_with(|| mxlsink::imp::InitialTime {
            index: current_index,
            gst_time,
        });
    let initial_info = state.initial_time.as_ref().ok_or(gst::FlowError::Error)?;

    let mut write_index = current_index;
    if let Some(pts) = buffer.pts() {
        let abs_pts = pts + initial_info.gst_time;
        write_index = state
            .instance
            .timestamp_to_index(abs_pts.nseconds(), &sample_rate)
            .map_err(|_| gst::FlowError::Error)?
            + initial_info.index;

        if write_index > current_index + buffer_length {
            write_index = current_index + buffer_length - 1;
        }
    }

    trace!(
        "Writing audio batch starting at index {}, sample_rate {}/{}",
        write_index,
        sample_rate.numerator,
        sample_rate.denominator
    );

    let max_chunk = (buffer_length / 2) as usize;
    let num_channels = audio_state.flow_def.channel_count as usize;
    let samples_total = samples_per_buffer;
    let mut remaining = samples_total;
    let mut src_offset_samples = 0;

    while remaining > 0 {
        let chunk_samples = remaining.min(max_chunk);
        let chunk_bytes = chunk_samples * num_channels * bytes_per_sample;

        let mut access = audio_state
            .writer
            .open_samples(write_index, chunk_samples as usize)
            .map_err(|_| gst::FlowError::Error)?;

        let samples_per_channel = chunk_samples;
        let src_chunk = &src[src_offset_samples * num_channels * bytes_per_sample
            ..src_offset_samples * num_channels * bytes_per_sample + chunk_bytes];

        for ch in 0..num_channels {
            let (plane1, plane2) = access
                .channel_data_mut(ch)
                .map_err(|_| gst::FlowError::Error)?;

            let mut written = 0;
            let offset = ch * bytes_per_sample;

            for i in 0..samples_per_channel {
                let sample_offset = i * num_channels * bytes_per_sample + offset;
                if sample_offset + bytes_per_sample > src_chunk.len() {
                    break;
                }

                if written + bytes_per_sample <= plane1.len() {
                    plane1[written..written + bytes_per_sample].copy_from_slice(
                        &src_chunk[sample_offset..sample_offset + bytes_per_sample],
                    );
                } else if written < plane1.len() + plane2.len() {
                    let plane2_offset = written.saturating_sub(plane1.len());
                    if plane2_offset + bytes_per_sample <= plane2.len() {
                        plane2[plane2_offset..plane2_offset + bytes_per_sample].copy_from_slice(
                            &src_chunk[sample_offset..sample_offset + bytes_per_sample],
                        );
                    }
                }

                written += bytes_per_sample;
            }
        }

        access.commit().map_err(|_| gst::FlowError::Error)?;
        trace!(
            "Committed chunk: {} samples at index {} ({} bytes)",
            chunk_samples,
            write_index,
            chunk_bytes
        );

        write_index = write_index.wrapping_add(chunk_samples as u64);
        src_offset_samples += chunk_samples;
        remaining -= chunk_samples;
    }

    Ok(gst::FlowSuccess::Ok)
}
