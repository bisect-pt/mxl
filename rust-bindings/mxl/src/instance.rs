use std::ffi::CString;

use dlopen2::wrapper::Container;

use crate::{Error, MxlApi, MxlFlowReader, Result};

pub struct MxlInstance {
    pub(crate) api: Container<MxlApi>,
    pub(crate) instance: mxl_sys::mxlInstance,
}

impl MxlInstance {
    pub fn new(api: Container<MxlApi>, domain: &str, options: &str) -> Result<Self> {
        let instance = unsafe {
            api.mxl_create_instance(
                CString::new(domain)?.as_ptr(),
                CString::new(options)?.as_ptr(),
            )
        };
        if instance.is_null() {
            Err(Error::Other("Failed to create MXL instance.".to_string()))
        } else {
            Ok(Self { api, instance })
        }
    }

    pub fn destroy(&mut self) -> Result<()> {
        let result;
        if self.instance.is_null() {
            return Err(Error::Other(
                "Internal instance not initialized.".to_string(),
            ));
        }
        unsafe {
            result = Error::from_status(self.api.mxl_destroy_instance(self.instance));
        }
        self.instance = std::ptr::null_mut();
        result
    }

    pub fn create_flow_reader(&self, flow_id: &str) -> Result<MxlFlowReader> {
        let flow_id = CString::new(flow_id).map_err(|_| Error::InvalidArg)?;
        let options = CString::new("").map_err(|_| Error::InvalidArg)?;
        let mut reader: mxl_sys::mxlFlowReader = std::ptr::null_mut();
        unsafe {
            Error::from_status(self.api.mxl_create_flow_reader(
                self.instance,
                flow_id.as_ptr(),
                options.as_ptr(),
                &mut reader,
            ))?;
        }
        if reader.is_null() {
            return Err(Error::Other("Failed to create flow reader.".to_string()));
        }
        Ok(MxlFlowReader {
            instance: self,
            reader,
        })
    }

    pub fn get_current_head_index(&self, rational: &mxl_sys::Rational) -> u64 {
        unsafe { self.api.mxl_get_current_head_index(rational) }
    }
}

impl Drop for MxlInstance {
    fn drop(&mut self) {
        if !self.instance.is_null() {
            if let Err(error) = self.destroy() {
                tracing::error!("Failed to destroy MXL instance: {:?}", error);
            }
        }
    }
}
