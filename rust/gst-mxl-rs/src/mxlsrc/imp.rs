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

use mxl::config::get_mxl_so_path;
use mxl::GrainReader;
use mxl::MxlFlowReader;
use mxl::MxlInstance;
use tracing::trace;

use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::u128;

use crate::mxlsrc;

const GET_GRAIN_TIMEOUT: Duration = Duration::from_secs(5);

const LATENCY: gst::ClockTime = gst::ClockTime::from_mseconds(100);

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "rssrc",
        gst::DebugColorFlags::empty(),
        Some("Rust MXL Source"),
    )
});

const DEFAULT_FLOW_ID: &str = "";
const DEFAULT_DOMAIN: &str = "";

#[derive(Debug, Clone)]
struct InitialTime {
    mxl_index: u64,
    gst_time: gst::ClockTime,
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
    pub format: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub framerate: Option<gst::Fraction>,
    pub interlace_mode: Option<String>,
    pub colorimetry: Option<String>,
    reader: Option<MxlFlowReader>,
    pub instance: Option<MxlInstance>,
    initial_info: Option<InitialTime>,
    frame_counter: u64,
    pub is_initialized: bool,
    pub grain_reader: Option<GrainReader>,
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
    settings: Mutex<Settings>,
    state: Mutex<State>,
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

        let obj = self.obj();
        obj.set_live(true);
        obj.set_format(gst::Format::Time);
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
            let caps_struct = gst::Structure::builder("video/x-raw")
                .field("format", "v210")
                .field("width", 1920)
                .field("height", 1080)
                .field("framerate", gst::Fraction::new(30000, 1001))
                .field("interlace-mode", "progressive")
                .field("colorimetry", "bt709")
                .build();

            let caps = gst::Caps::builder_full().structure(caps_struct).build();

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
    fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| gst::loggable_error!(CAT, "Failed to lock state mutex: {}", e))?;

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
            .unwrap_or_else(|_| "bt709".to_string());

        state.format = Some(format);
        state.width = Some(width);
        state.height = Some(height);
        state.framerate = Some(framerate);
        state.interlace_mode = Some(interlace_mode);
        state.colorimetry = Some(colorimetry);

        gst::info!(
            CAT,
            imp = self,
            "Negotiated caps: format={} {}x{} @ {}/{}fps, interlace={}, colorimetry={}",
            state.format.as_deref().unwrap_or("unknown"),
            width,
            height,
            framerate.numer(),
            framerate.denom(),
            state.interlace_mode.as_deref().unwrap_or("unknown"),
            state.colorimetry.as_deref().unwrap_or("unknown"),
        );

        Ok(())
    }

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
        let reader = init_mxl_reader(&settings)?;
        state.reader = Some(reader);
        state.is_initialized = false;
        gst::info!(CAT, imp = self, "Started");

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        *self.state.lock().map_err(|e| {
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

    fn fixate(&self, mut caps: gst::Caps) -> gst::Caps {
        caps.truncate();
        {
            let caps = caps.make_mut();
            if let Some(s) = caps.structure_mut(0) {
                s.set("format", "v210");
                s.set("width", 1920);
                s.set("height", 1080);
                s.set("framerate", gst::Fraction::new(30000, 1001));
                s.set("interlace-mode", "progressive");
                s.set("colorimetry", "bt709");
            }
        }

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

fn init_mxl_reader(
    settings: &MutexGuard<'_, Settings>,
) -> Result<MxlFlowReader, gst::ErrorMessage> {
    let mxl_instance = init_mxl_instance(settings)?;

    let reader = mxl_instance
        .create_flow_reader(settings.flow_id.as_str())
        .map_err(|e| {
            gst::error_msg!(
                gst::CoreError::Failed,
                ["Failed to create MXL reader: {}", e]
            )
        })?;
    Ok(reader)
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

impl PushSrcImpl for MxlSrc {
    fn create(
        &self,
        _buffer: Option<&mut gst::BufferRef>,
    ) -> Result<CreateSuccess, gst::FlowError> {
        let mut state = self.state.lock().map_err(|_| gst::FlowError::Error)?;
        let settings = self.settings.lock().map_err(|_| gst::FlowError::Error)?;
        if state.reader.is_none() {
            state.reader = Some(init_mxl_reader(&settings).map_err(|_| gst::FlowError::Error)?);
        }
        if state.instance.is_none() {
            state.instance = Some(init_mxl_instance(&settings).map_err(|_| gst::FlowError::Error)?);
        }
        let current_index;
        let rate;
        {
            let reader = state
                .reader
                .as_ref()
                .ok_or_else(|| init_mxl_reader(&settings))
                .map_err(|_| gst::FlowError::Error)?;
            let binding = reader.get_info();
            let reader_info = binding.as_ref();
            rate = reader_info
                .map_err(|_| gst::FlowError::Error)?
                .discrete_flow_info()
                .map_err(|_| gst::FlowError::Error)?
                .grainRate;
            current_index = state
                .instance
                .as_ref()
                .ok_or(gst::FlowError::Error)?
                .get_current_index(&rate);
        }
        if state.grain_reader.is_none() {
            state.grain_reader = Some(
                init_mxl_reader(&settings)
                    .map_err(|_| gst::FlowError::Error)?
                    .to_grain_reader()
                    .map_err(|_| gst::FlowError::Error)?,
            );
        };
        let Some(ts_gst) = self.obj().current_running_time() else {
            return Err(gst::FlowError::Error);
        };
        if !state.is_initialized {
            state.initial_info = Some(InitialTime {
                mxl_index: current_index,
                gst_time: ts_gst,
            });
            state.is_initialized = true;
        }

        let initial_info = state.initial_info.as_ref().ok_or(gst::FlowError::Error)?;

        let mut next_frame_index = initial_info.mxl_index + state.frame_counter;

        let grain_request_time = Instant::now();
        let real_time_start = SystemTime::now();
        let mut frames_ahead = 0;
        if next_frame_index < current_index {
            let missed_frames = current_index - next_frame_index;
            trace!(
                "Skipped frames! next_frame_index={} < head_index={} (lagging {})",
                next_frame_index,
                current_index,
                missed_frames
            );
            next_frame_index += missed_frames;
        } else if next_frame_index > current_index {
            frames_ahead = next_frame_index - current_index;
            trace!("index={} > head_index={}", next_frame_index, current_index);
            next_frame_index -= frames_ahead;
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
        let pts = (state.frame_counter - frames_ahead) as u128 * 1_000_000_000u128;
        let pts = pts * rate.denominator as u128;
        let pts = pts / rate.numerator as u128;

        let pts = gst::ClockTime::from_nseconds(pts as u64);

        let mut pts = pts + initial_info.gst_time;
        let _ = initial_info;
        let initial_info = state.initial_info.as_mut().ok_or(gst::FlowError::Error)?;
        if pts < ts_gst {
            let prev_pts = pts;
            pts = pts - initial_info.gst_time;
            initial_info.gst_time = initial_info.gst_time + ts_gst - prev_pts;
            pts = pts + initial_info.gst_time;
        }
        let pts = pts + LATENCY;
        let mut buffer;
        {
            let binding = state.grain_reader.as_ref().ok_or(gst::FlowError::Error)?;
            trace!("Getting grain with index: {}", next_frame_index);
            let grain_data = match binding.get_complete_grain(next_frame_index, GET_GRAIN_TIMEOUT) {
                Ok(r) => r,

                Err(err) => {
                    trace!("error: {err}");
                    return Err(gst::FlowError::Error);
                }
            };

            buffer = gst::Buffer::with_size(grain_data.payload.len())
                .map_err(|_| gst::FlowError::Error)?;

            {
                let buffer = buffer.get_mut().ok_or(gst::FlowError::Error)?;
                buffer.set_pts(pts);
                let mut map = buffer.map_writable().map_err(|_| gst::FlowError::Error)?;
                map.as_mut_slice().copy_from_slice(grain_data.payload);
            }
        }

        trace!("PTS: {:?} GST-CURRENT: {:?}", buffer.pts(), ts_gst);
        trace!("Produced buffer {:?}", buffer);
        if state.frame_counter == 0 {
            state.frame_counter += 2;
        } else {
            state.frame_counter += 1;
        }
        Ok(CreateSuccess::NewBuffer(buffer))
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use gst::CoreError;

    use super::*;

    #[test]
    fn set_properties() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let element = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "test_flow")
            .property("domain", "mydomain")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let flow_id: String = element.property("flow-id");
        let domain: String = element.property("domain");

        assert_eq!(flow_id, "test_flow");
        assert_eq!(domain, "mydomain");
        Ok(())
    }

    #[test]
    #[ignore]
    #[tracing_test::traced_test]
    fn negotiate_caps() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let factory = gst::ElementFactory::find("mxlsrc").expect("mxlsrc not registered");
        let pad_templates = factory.static_pad_templates();
        assert!(!pad_templates.is_empty());

        let src_templ = pad_templates
            .iter()
            .find(|t| t.direction() == gst::PadDirection::Src)
            .ok_or(gst::CoreError::Failed)
            .map_err(|_| glib::Error::new(CoreError::Pad, "Pad templates failed"))?;
        trace!("Advertised caps: {}", src_templ.caps());

        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "9fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let queue1 = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let queue2 = gst::ElementFactory::make("queue")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        let sink = gst::ElementFactory::make("autovideosink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .add_many(&[&src, &queue1, &convert, &queue2, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        gst::Element::link_many([&src, &queue1, &convert, &queue2, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;

        let src_pad = src
            .static_pad("src")
            .ok_or(CoreError::Failed)
            .map_err(|_| glib::Error::new(CoreError::Pad, "Source pad failed"))?;
        if let Some(caps) = src_pad.current_caps() {
            trace!("Negotiated caps: {}", caps.to_string());
        } else {
            trace!("No negotiated caps found");
        }
        std::thread::sleep(std::time::Duration::from_millis(100000));
        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        Ok(())
    }

    #[test]
    #[ignore]
    #[tracing_test::traced_test]
    fn start_valid_pipeline() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let src = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "5fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let sink = gst::ElementFactory::make("fakesink")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many(&[&src, &sink])
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;
        src.link(&sink)
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        thread::sleep(Duration::from_millis(600));
        pipeline
            .set_state(gst::State::Null)
            .map_err(|_| glib::Error::new(CoreError::Failed, "State change failed"))?;
        Ok(())
    }

    #[test]
    #[ignore]
    fn is_valid_reader() -> Result<(), glib::Error> {
        gst::init()?;
        gst::Element::register(None, "mxlsrc", gst::Rank::NONE, MxlSrc::type_())
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let element = gst::ElementFactory::make("mxlsrc")
            .property("flow-id", "5fbec3b1-1b0f-417d-9059-8b94a47197ed")
            .property("domain", "/mnt/mxl/domain_1")
            .build()
            .map_err(|e| glib::Error::new(CoreError::Failed, &e.message))?;

        let flow_id: String = element.property("flow-id");
        let domain: String = element.property("domain");
        let mxl_api = mxl::load_api(get_mxl_so_path())
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;

        let mxl_instance = mxl::MxlInstance::new(mxl_api, domain.as_str(), "")
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;

        let reader = mxl_instance
            .create_flow_reader(flow_id.as_str())
            .map_err(|e| glib::Error::new(gst::CoreError::Failed, e.to_string().as_str()))?;
        assert!(reader.get_info().is_ok());
        Ok(())
    }
}
