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
use gst_base::prelude::*;
use gst_base::subclass::base_src::CreateSuccess;
use gst_base::subclass::prelude::*;

use mxl::GrainReader;
use mxl::MxlFlowReader;
use mxl::MxlInstance;
use mxl::Rational;
use mxl::SamplesReader;
use tracing::trace;

use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::u128;

use crate::mxlsrc;
use crate::mxlsrc::mxl_helper;

const GET_GRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_BATCH_SIZE: u32 = 48;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "rssrc",
        gst::DebugColorFlags::empty(),
        Some("Rust MXL Source"),
    )
});

const DEFAULT_FLOW_ID: &str = "";
const DEFAULT_DOMAIN: &str = "";

#[derive(Debug, Default, Clone)]
pub struct InitialTime {
    pub mxl_index: u64,
    pub gst_time: gst::ClockTime,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub video_flow: Option<String>,
    pub audio_flow: Option<String>,
    pub domain: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            video_flow: None,
            audio_flow: None,
            domain: DEFAULT_DOMAIN.to_owned(),
        }
    }
}

pub struct State {
    pub instance: MxlInstance,
    pub initial_info: InitialTime,
    pub video: Option<VideoState>,
    pub audio: Option<AudioState>,
}

pub struct VideoState {
    pub grain_rate: Rational,
    pub frame_counter: u64,
    pub is_initialized: bool,
    pub grain_reader: GrainReader,
}

pub struct AudioState {
    pub reader: MxlFlowReader,
    pub samples_reader: SamplesReader,
    pub batch_counter: u64,
    pub is_initialized: bool,
    pub index: u64,
    pub next_discont: bool,
}

#[derive(Default)]
pub struct Context {
    pub state: Option<State>,
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
pub struct MxlSrc {
    pub settings: Mutex<Settings>,
    pub context: Mutex<Context>,
    clock_wait: Mutex<ClockWait>,
}

#[glib::object_subclass]
impl ObjectSubclass for MxlSrc {
    const NAME: &'static str = "GstRsMxlSrc";
    type Type = mxlsrc::MxlSrc;
    type ParentType = gst_base::PushSrc;
}

impl ObjectImpl for MxlSrc {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
            vec![
                glib::ParamSpecString::builder("video-flow")
                    .nick("VideoFlowID")
                    .blurb("Video Flow ID")
                    .default_value(DEFAULT_FLOW_ID)
                    .mutable_ready()
                    .build(),
                glib::ParamSpecString::builder("audio-flow")
                    .nick("AudioFlowID")
                    .blurb("Audio Flow ID")
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
        let obj = self.obj();
        obj.set_live(true);
        obj.set_format(gst::Format::Time);
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        if let Ok(mut settings) = self.settings.lock() {
            match pspec.name() {
                "video-flow" => {
                    if let Ok(flow_id) = value.get::<String>() {
                        settings.video_flow = Some(flow_id);
                    } else {
                        gst::error!(CAT, imp = self, "Invalid type for video-flow property");
                    }
                }
                "audio-flow" => {
                    if let Ok(flow_id) = value.get::<String>() {
                        settings.audio_flow = Some(flow_id);
                    } else {
                        gst::error!(CAT, imp = self, "Invalid type for audio-flow property");
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
                "video-flow" => settings.video_flow.to_value(),
                "audio-flow" => settings.video_flow.to_value(),
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

impl GstObjectImpl for MxlSrc {}

impl ElementImpl for MxlSrc {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Gstreamer MXL Source",
                "Source/Video",
                "Creates video flow",
                "Bisect",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        use std::sync::LazyLock;

        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst::Caps::new_any();
            let src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &caps,
            )
            .expect("Failed to create src pad template");

            vec![src_pad_template]
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

impl BaseSrcImpl for MxlSrc {
    fn event(&self, event: &gst::Event) -> bool {
        self.parent_event(event)
    }

    fn negotiate(&self) -> Result<(), gst::LoggableError> {
        gst::info!(CAT, imp = self, "Negotiating caps…");

        let settings = self
            .settings
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock settings mutex {}", e))?;
        if settings.audio_flow.is_some() && settings.video_flow.is_some() {
            gst::warning!(CAT, imp = self, "You can't set both video and audio flows");
            return self.parent_negotiate();
        }
        if settings.domain.is_empty()
            || settings.video_flow.is_none() && settings.audio_flow.is_none()
        {
            gst::warning!(CAT, imp = self, "domain or flow-id not set yet");
            return self.parent_negotiate();
        }

        let flow_id = mxl_helper::get_flow_type_id(&settings)?;
        let serde_json = mxl_helper::get_mxl_flow_json(&settings.domain, flow_id)?;
        let json_def = mxl_helper::get_flow_def(self, serde_json)?;
        mxl_helper::set_json_caps(self, json_def)
    }

    fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let structure = caps
            .structure(0)
            .ok_or_else(|| gst::loggable_error!(CAT, "No structure in caps {}", caps))?;

        let format = structure
            .get::<String>("format")
            .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;
        if format == "v210" {
            let width = structure
                .get::<i32>("width")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;
            let height = structure
                .get::<i32>("height")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;
            let framerate = structure
                .get::<gst::Fraction>("framerate")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;
            let interlace_mode = structure
                .get::<String>("interlace-mode")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;
            let colorimetry = structure
                .get::<String>("colorimetry")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to set caps {}", e))?;

            trace!(
                "Negotiated caps: format={} {}x{} @ {}/{}fps, interlace={}, colorimetry={}",
                format,
                width,
                height,
                framerate.numer(),
                framerate.denom(),
                interlace_mode,
                colorimetry,
            );

            return Ok(());
        } else if format == "F32LE" {
            let rate = structure
                .get::<i32>("rate")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to get rate from caps: {}", e))?;

            let channels = structure.get::<i32>("channels").map_err(|e| {
                gst::loggable_error!(CAT, "Failed to get channels from caps: {}", e)
            })?;

            let format = structure
                .get::<String>("format")
                .map_err(|e| gst::loggable_error!(CAT, "Failed to get format from caps: {}", e))?;
            trace!(
                "Negotiated caps: format={}, rate={}, channel_count={} ",
                format,
                rate,
                channels
            );
            return Ok(());
        }
        Err(gst::loggable_error!(
            CAT,
            "Failed to set caps: No valid format"
        ))
    }

    fn start(&self) -> Result<(), gst::ErrorMessage> {
        self.unlock_stop()?;
        mxl_helper::init(self)?;
        gst::info!(CAT, imp = self, "Started");

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        *self.context.lock().map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to get settings mutex: {}", e]
            )
        })? = Default::default();
        self.unlock()?;

        gst::info!(CAT, imp = self, "Stopped");

        Ok(())
    }

    fn query(&self, query: &mut gst::QueryRef) -> bool {
        BaseSrcImplExt::parent_query(self, query)
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
}

impl PushSrcImpl for MxlSrc {
    fn create(
        &self,
        _buffer: Option<&mut gst::BufferRef>,
    ) -> Result<CreateSuccess, gst::FlowError> {
        let mut context = self.context.lock().map_err(|_| gst::FlowError::Error)?;
        let state = context.state.as_mut().ok_or(gst::FlowError::Error)?;
        if state.video.is_some() {
            create_video(self, state)
        } else if state.audio.is_some() {
            create_audio(self, state)
        } else {
            Err(gst::FlowError::Error)
        }
    }
}

fn create_video(src: &MxlSrc, state: &mut State) -> Result<CreateSuccess, gst::FlowError> {
    let video_state = state.video.as_mut().ok_or(gst::FlowError::Error)?;
    let current_index;
    let rate = video_state.grain_rate;
    {
        current_index = state.instance.get_current_index(&rate);
    }
    let Some(ts_gst) = src.obj().current_running_time() else {
        return Err(gst::FlowError::Error);
    };
    if !video_state.is_initialized {
        state.initial_info = InitialTime {
            mxl_index: current_index,
            gst_time: ts_gst,
        };
        video_state.is_initialized = true;
    }

    let initial_info = &state.initial_info;

    let mut next_frame_index = initial_info.mxl_index + video_state.frame_counter;
    let _ = initial_info;
    let initial_info = &state.initial_info;
    let grain_request_time = Instant::now();
    let real_time_start = SystemTime::now();
    if next_frame_index < current_index {
        let missed_frames = current_index - next_frame_index;
        trace!(
            "Skipped frames! next_frame_index={} < head_index={} (lagging {})",
            next_frame_index,
            current_index,
            missed_frames
        );
        next_frame_index = current_index;
    } else if next_frame_index > current_index {
        let frames_ahead = next_frame_index - current_index;
        trace!(
            "index={} > head_index={} (ahead {} frames)",
            next_frame_index,
            current_index,
            frames_ahead
        );
    }
    let real_time_end = SystemTime::now();
    let elapsed_real = real_time_end
        .duration_since(real_time_start)
        .unwrap_or_default();

    let start = real_time_start
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let end = real_time_end
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let start_hms = {
        let total_secs = start.as_secs();
        let hours = total_secs / 3600 % 24;
        let minutes = total_secs / 60 % 60;
        let seconds = total_secs % 60;
        let millis = start.subsec_millis();
        format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
    };

    let end_hms = {
        let total_secs = end.as_secs();
        let hours = total_secs / 3600 % 24;
        let minutes = total_secs / 60 % 60;
        let seconds = total_secs % 60;
        let millis = end.subsec_millis();
        format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
    };

    trace!(
                "Grain number: {} | Grain request time: {} µs | Real time start: {} | Real time end: {} | Elapsed wall time: {} ms",
                next_frame_index,
                grain_request_time.elapsed().as_micros(),
                start_hms,
                end_hms,
                elapsed_real.as_millis()
            );
    let _ = initial_info;
    let initial_info = &state.initial_info;
    let pts = (video_state.frame_counter) as u128 * 1_000_000_000u128;
    let pts = pts * rate.denominator as u128;
    let pts = pts / rate.numerator as u128;

    let pts = gst::ClockTime::from_nseconds(pts as u64);

    let mut pts = pts + initial_info.gst_time;
    let _ = initial_info;
    let initial_info = &mut state.initial_info;
    if pts < ts_gst {
        let prev_pts = pts;
        pts = pts - initial_info.gst_time;
        initial_info.gst_time = initial_info.gst_time + ts_gst - prev_pts;
        pts = pts + initial_info.gst_time;
    }

    let mut buffer;
    {
        let binding = &video_state.grain_reader;
        trace!("Getting grain with index: {}", next_frame_index);
        let grain_data = match binding.get_complete_grain(next_frame_index, GET_GRAIN_TIMEOUT) {
            Ok(r) => r,

            Err(err) => {
                trace!("error: {err}");
                return Err(gst::FlowError::Error);
            }
        };

        buffer =
            gst::Buffer::with_size(grain_data.payload.len()).map_err(|_| gst::FlowError::Error)?;

        {
            let buffer = buffer.get_mut().ok_or(gst::FlowError::Error)?;
            buffer.set_pts(pts);
            let mut map = buffer.map_writable().map_err(|_| gst::FlowError::Error)?;
            map.as_mut_slice().copy_from_slice(grain_data.payload);
        }
    }

    trace!("PTS: {:?} GST-CURRENT: {:?}", buffer.pts(), ts_gst);
    trace!("Produced buffer {:?}", buffer);
    if video_state.frame_counter == 0 {
        video_state.frame_counter += 2;
    } else {
        video_state.frame_counter += 1;
    }
    Ok(CreateSuccess::NewBuffer(buffer))
}

fn create_audio(src: &MxlSrc, state: &mut State) -> Result<CreateSuccess, gst::FlowError> {
    let audio_state = state.audio.as_mut().ok_or(gst::FlowError::Error)?;
    let mut reader_info = audio_state
        .reader
        .get_info()
        .map_err(|_| gst::FlowError::Error)?;
    let mut reader_info_cont = reader_info
        .continuous_flow_info()
        .map_err(|_| gst::FlowError::Error)?;
    let sample_rate = reader_info_cont.sampleRate;

    let batch_size = DEFAULT_BATCH_SIZE.min(reader_info_cont.bufferLength / 2);
    let ring = reader_info_cont.bufferLength as u64;
    let batch = batch_size as u64;

    let Some(ts_gst) = src.obj().current_running_time() else {
        return Err(gst::FlowError::Error);
    };

    if !audio_state.is_initialized {
        state.initial_info = InitialTime {
            mxl_index: state.instance.get_time(),
            gst_time: ts_gst,
        };
        audio_state.index = reader_info_cont.headIndex.saturating_sub(batch);
        audio_state.is_initialized = true;
        audio_state.batch_counter = 0;
    }

    let mut head = reader_info_cont.headIndex as u64;
    while audio_state.index + batch > head {
        trace!(
            "Reader ahead: index {} + batch {} > head {} (waiting for producer)",
            audio_state.index,
            batch,
            head
        );
        reader_info = audio_state
            .reader
            .get_info()
            .map_err(|_| gst::FlowError::Error)?;
        reader_info_cont = reader_info
            .continuous_flow_info()
            .map_err(|_| gst::FlowError::Error)?;
        head = reader_info_cont.headIndex as u64;
    }

    let oldest_valid = head.saturating_sub(ring.saturating_sub(batch));
    if audio_state.index < oldest_valid {
        let cushion = batch.saturating_mul(2);
        let target = head.saturating_sub(cushion);
        trace!(
            "CATCH-UP (pre-read): index {} < oldest {}. Jumping -> {}, head={}, ring={}",
            audio_state.index,
            oldest_valid,
            target,
            head,
            ring
        );

        audio_state.index = target;

        state.initial_info.gst_time = ts_gst;
        state.initial_info.mxl_index = state.instance.get_time();
        audio_state.batch_counter = 0;
        audio_state.next_discont = true;
    }

    let read_once = |idx: u64| audio_state.samples_reader.get_samples(idx, batch as usize);

    let samples = match read_once(audio_state.index) {
        Ok(s) => s,
        Err(_) => {
            reader_info = audio_state
                .reader
                .get_info()
                .map_err(|_| gst::FlowError::Error)?;
            reader_info_cont = reader_info
                .continuous_flow_info()
                .map_err(|_| gst::FlowError::Error)?;
            head = reader_info_cont.headIndex as u64;

            let cushion = batch.saturating_mul(2);
            let target = head.saturating_sub(cushion);
            trace!(
                "CATCH-UP (retry): get_samples failed at {}, head {}. Jumping -> {}",
                audio_state.index,
                head,
                target
            );

            audio_state.index = target;
            state.initial_info.gst_time = ts_gst;
            state.initial_info.mxl_index = state.instance.get_time();
            audio_state.batch_counter = 0;
            audio_state.next_discont = true;

            read_once(audio_state.index).map_err(|_| gst::FlowError::Error)?
        }
    };
    let num_channels = samples.num_of_channels();
    let mut channels: Vec<Vec<u8>> = Vec::with_capacity(num_channels);
    let mut total_samples_per_channel = 0;

    for ch in 0..num_channels {
        let (data1, data2) = samples
            .channel_data(ch)
            .map_err(|_| gst::FlowError::Error)?;
        let mut combined = Vec::with_capacity(data1.len() + data2.len());
        combined.extend_from_slice(data1);
        combined.extend_from_slice(data2);
        total_samples_per_channel = combined.len() / std::mem::size_of::<f32>();
        channels.push(combined);
    }
    let mut interleaved =
        Vec::with_capacity(total_samples_per_channel * num_channels * std::mem::size_of::<f32>());
    for frame in 0..total_samples_per_channel {
        for ch in 0..num_channels {
            let chan = &channels[ch];
            let offset = frame * std::mem::size_of::<f32>();
            interleaved.extend_from_slice(&chan[offset..offset + std::mem::size_of::<f32>()]);
        }
    }

    let next_index = audio_state.index + batch;
    let next_head_timestamp = state
        .instance
        .index_to_timestamp(next_index, &sample_rate)
        .map_err(|_| gst::FlowError::Error)?;
    let read_head_timestamp = state
        .instance
        .index_to_timestamp(audio_state.index, &sample_rate)
        .map_err(|_| gst::FlowError::Error)?;
    let read_batch_duration = next_head_timestamp - read_head_timestamp;

    state.initial_info.mxl_index = state
        .initial_info
        .mxl_index
        .saturating_add(read_batch_duration);

    let now_mxl = state.instance.get_time();
    let sleep_ns = state.initial_info.mxl_index.saturating_sub(now_mxl);
    let sleep_duration = Duration::from_nanos(sleep_ns);
    if !sleep_duration.is_zero() {
        trace!("Will sleep for {:?}.", sleep_duration);
        state.instance.sleep_for(sleep_duration);
    }

    let batch_duration_ns = (batch as u128 * 1_000_000_000u128) * sample_rate.denominator as u128
        / sample_rate.numerator as u128;

    let pts_ns = gst::ClockTime::from_nseconds(
        (audio_state.batch_counter as u128 * batch_duration_ns) as u64,
    );
    let mut pts = state.initial_info.gst_time + pts_ns;

    if pts < ts_gst {
        state.initial_info.gst_time += ts_gst - pts;
        pts = ts_gst;
    }

    let mut buf_size = 0;
    for i in 0..samples.num_of_channels() {
        let (a, b) = samples.channel_data(i).map_err(|_| gst::FlowError::Error)?;
        buf_size += a.len() + b.len();
    }

    let mut buffer = gst::Buffer::with_size(buf_size).map_err(|_| gst::FlowError::Error)?;

    {
        let buffer = buffer.get_mut().ok_or(gst::FlowError::Error)?;
        buffer.set_pts(pts);

        if std::mem::take(&mut audio_state.next_discont) {
            buffer.set_flags(gst::BufferFlags::DISCONT);
        }

        let mut map = buffer.map_writable().map_err(|_| gst::FlowError::Error)?;
        map.as_mut_slice().copy_from_slice(&interleaved);
    }

    audio_state.batch_counter += 1;
    audio_state.index += batch;

    trace!(
        "Initial time: {} buffer PTS: {:?} gst running time: {}",
        state.initial_info.gst_time,
        pts,
        ts_gst
    );

    Ok(CreateSuccess::NewBuffer(buffer))
}
