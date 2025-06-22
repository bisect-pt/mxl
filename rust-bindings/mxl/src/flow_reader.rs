use std::time::Duration;

use crate::{
    Error, MxlInstance, Result,
    flow::{FlowInfo, GrainData},
};

pub struct MxlFlowReader<'a> {
    pub(crate) instance: &'a MxlInstance,
    pub(crate) reader: mxl_sys::mxlFlowReader,
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
