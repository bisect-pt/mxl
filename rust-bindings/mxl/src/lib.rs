mod api;
mod error;
mod instance;

pub use api::{MxlApi, load_api};
pub use error::{Error, Result};
pub use instance::MxlInstance;

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Unspecified,
    Video,
    Audio,
    Data,
    Mux,
}

impl From<u32> for DataFormat {
    fn from(value: u32) -> Self {
        match value {
            0 => DataFormat::Unspecified,
            1 => DataFormat::Video,
            2 => DataFormat::Audio,
            3 => DataFormat::Data,
            4 => DataFormat::Mux,
            _ => DataFormat::Unspecified,
        }
    }
}

pub struct FlowInfo {
    value: mxl_sys::FlowInfo,
}

impl FlowInfo {
    pub fn discrete_flow_info(&self) -> Result<&mxl_sys::DiscreteFlowInfo> {
        // Check is based on mxlIsDiscreteDataFormat, which is inline, thus not accessible in
        // mxl_sys.
        if self.value.common.format != mxl_sys::MXL_DATA_FORMAT_VIDEO
            && self.value.common.format != mxl_sys::MXL_DATA_FORMAT_DATA
        {
            return Err(Error::Other(format!(
                "Flow format is {}, video or data require.",
                self.value.common.format
            )));
        }
        Ok(unsafe { &self.value.__bindgen_anon_1.discrete })
    }
}

pub struct GrainData {
    pub user_data: Vec<u8>,
    pub payload: Vec<u8>,
}

pub struct MxlFlowReader<'a> {
    instance: &'a MxlInstance,
    reader: mxl_sys::mxlFlowReader,
}

impl<'a> MxlFlowReader<'a> {
    pub fn destroy(&mut self) -> Result<()> {
        let result;
        if self.reader.is_null() {
            return Err(Error::InvalidArg);
        }
        unsafe {
            result = Error::from_status(
                self.instance
                    .api
                    .mxl_release_flow_reader(self.instance.instance, self.reader),
            );
        }
        self.reader = std::ptr::null_mut();
        result
    }

    pub fn get_info(&self) -> Result<FlowInfo> {
        let mut flow_info: mxl_sys::FlowInfo = unsafe { std::mem::zeroed() };
        unsafe {
            Error::from_status(
                self.instance
                    .api
                    .mxl_flow_reader_get_info(self.reader, &mut flow_info),
            )?;
        }
        Ok(FlowInfo { value: flow_info })
    }

    pub fn get_complete_grain(&self, index: u64, timeout: Duration) -> Result<GrainData> {
        let mut grain_info: mxl_sys::GrainInfo = unsafe { std::mem::zeroed() };
        let mut payload_ptr: *mut u8 = std::ptr::null_mut();
        let timeout_ns = timeout.as_nanos() as u64;
        loop {
            unsafe {
                Error::from_status(self.instance.api.mxl_flow_reader_get_grain(
                    self.reader,
                    index,
                    timeout_ns,
                    &mut grain_info,
                    &mut payload_ptr,
                ))?;
            }
            if grain_info.commitedSize != grain_info.grainSize {
                // We don't need partial grains. Wait for the grain to be complete.
                continue;
            }
            if payload_ptr.is_null() {
                return Err(Error::Other(format!(
                    "Failed to get grain payload for index {}.",
                    index
                )));
            }
            break;
        }
        let payload = unsafe {
            let slice = std::slice::from_raw_parts(payload_ptr, grain_info.grainSize as usize);
            slice.to_vec()
        };
        let user_data = grain_info.userData.to_vec();
        Ok(GrainData { user_data, payload })
    }
}

impl Drop for MxlFlowReader<'_> {
    fn drop(&mut self) {
        if !self.reader.is_null() {
            if let Err(error) = self.destroy() {
                tracing::error!("Failed to release MXL flow reader: {:?}", error);
            }
        }
    }
}
