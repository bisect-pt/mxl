use std::time::Instant;

use glib::subclass::types::ObjectSubclassExt;
use gst::{prelude::*, ClockTime};
use tracing::trace;

use crate::mxlsink::{self, imp::*};

pub(crate) fn video(
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

pub(crate) fn audio(
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
