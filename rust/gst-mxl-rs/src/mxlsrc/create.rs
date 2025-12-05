use std::time::{Instant, SystemTime};

use crate::mxlsrc::imp::MxlSrc;
use crate::mxlsrc::state::{InitialTime, State};
use glib::subclass::types::ObjectSubclassExt;
use gst::prelude::*;
use gst_base::subclass::base_src::CreateSuccess;
use std::time::Duration;
use tracing::trace;

const GET_GRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_BATCH_SIZE: u32 = 48;

pub(crate) fn create_video(
    src: &MxlSrc,
    state: &mut State,
) -> Result<CreateSuccess, gst::FlowError> {
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

pub(crate) fn create_audio(
    src: &MxlSrc,
    state: &mut State,
) -> Result<CreateSuccess, gst::FlowError> {
    let audio_state = state.audio.as_mut().ok_or(gst::FlowError::Error)?;
    let mut reader_info = audio_state
        .reader
        .get_info()
        .map_err(|_| gst::FlowError::Error)?;
    let reader_info_cont = reader_info
        .config
        .continuous()
        .map_err(|_| gst::FlowError::Error)?;
    let sample_rate = reader_info
        .config
        .common()
        .sample_rate()
        .map_err(|_| gst::FlowError::Error)?;

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
        audio_state.index = reader_info.runtime.head_index().saturating_sub(batch);
        audio_state.is_initialized = true;
        audio_state.batch_counter = 0;
    }

    let mut head = reader_info.runtime.head_index() as u64;
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
        head = reader_info.runtime.head_index() as u64;
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

    let read_once = |idx: u64| {
        audio_state
            .samples_reader
            .get_samples_non_blocking(idx, batch as usize)
    };

    let samples = match read_once(audio_state.index) {
        Ok(s) => s,
        Err(_) => {
            reader_info = audio_state
                .reader
                .get_info()
                .map_err(|_| gst::FlowError::Error)?;
            head = reader_info.runtime.head_index() as u64;

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
