use std::sync::Arc;

use crate::flow_reader::get_flow_info;
use crate::{Error, Result, flow::FlowInfo, instance::InstanceContext};

pub struct SamplesReader {
    context: Arc<InstanceContext>,
    reader: mxl_sys::mxlFlowReader,
}

impl SamplesReader {
    pub(crate) fn new(context: Arc<InstanceContext>, reader: mxl_sys::mxlFlowReader) -> Self {
        Self { context, reader }
    }

    pub fn destroy(mut self) -> Result<()> {
        self.destroy_inner()
    }

    pub fn get_info(&self) -> Result<FlowInfo> {
        get_flow_info(&self.context, self.reader)
    }

    fn destroy_inner(&mut self) -> Result<()> {
        if self.reader.is_null() {
            return Err(Error::InvalidArg);
        }

        let mut reader = std::ptr::null_mut();
        std::mem::swap(&mut self.reader, &mut reader);

        Error::from_status(unsafe {
            self.context
                .api
                .mxl_release_flow_reader(self.context.instance, reader)
        })
    }
}

impl Drop for SamplesReader {
    fn drop(&mut self) {
        if !self.reader.is_null() {
            if let Err(err) = self.destroy_inner() {
                tracing::error!("Failed to release MXL flow reader (continuous): {:?}", err);
            }
        }
    }
}
