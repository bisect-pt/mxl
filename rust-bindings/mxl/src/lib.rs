use std::ffi::CString;
use std::path::Path;
use std::time::Duration;

use dlopen2::wrapper::{Container, WrapperApi};

#[derive(Debug, thiserror::Error)]
pub enum MxlError {
    #[error("Unknown error")]
    Unknown,
    #[error("Flow not found")]
    FlowNotFound,
    #[error("Out of range - too late")]
    OutOfRangeTooLate,
    #[error("Out of range - too early")]
    OutOfRangeTooEarly,
    #[error("Invalid flow reader")]
    InvalidFlowReader,
    #[error("Invalid flow writer")]
    InvalidFlowWriter,
    #[error("Timeout")]
    Timeout,
    #[error("Invalid argument")]
    InvalidArg,
    #[error("Conflict")]
    Conflict,
    /// The error is not defined in the MXL API, but it is used to wrap other errors.
    #[error("Other error: {0}")]
    Other(String),
}

fn status_to_result(status: mxl_sys::mxlStatus) -> Result<(), MxlError> {
    match status {
        mxl_sys::MXL_STATUS_OK => Ok(()),
        mxl_sys::MXL_ERR_UNKNOWN => Err(MxlError::Unknown),
        mxl_sys::MXL_ERR_FLOW_NOT_FOUND => Err(MxlError::FlowNotFound),
        mxl_sys::MXL_ERR_OUT_OF_RANGE_TOO_LATE => Err(MxlError::OutOfRangeTooLate),
        mxl_sys::MXL_ERR_OUT_OF_RANGE_TOO_EARLY => Err(MxlError::OutOfRangeTooEarly),
        mxl_sys::MXL_ERR_INVALID_FLOW_READER => Err(MxlError::InvalidFlowReader),
        mxl_sys::MXL_ERR_INVALID_FLOW_WRITER => Err(MxlError::InvalidFlowWriter),
        mxl_sys::MXL_ERR_TIMEOUT => Err(MxlError::Timeout),
        mxl_sys::MXL_ERR_INVALID_ARG => Err(MxlError::InvalidArg),
        mxl_sys::MXL_ERR_CONFLICT => Err(MxlError::Conflict),
        _ => Err(MxlError::Unknown),
    }
}

#[derive(WrapperApi)]
pub struct MxlApi {
    #[dlopen2_name = "mxlGetVersion"]
    mxl_get_version:
        unsafe extern "C" fn(out_version: *mut mxl_sys::mxlVersionType) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlCreateInstance"]
    mxl_create_instance: unsafe extern "C" fn(
        in_mxlDomain: *const std::os::raw::c_char,
        in_options: *const std::os::raw::c_char,
    ) -> mxl_sys::mxlInstance,
    #[dlopen2_name = "mxlGarbageCollectFlows"]
    mxl_garbage_collect_flows:
        unsafe extern "C" fn(in_instance: mxl_sys::mxlInstance) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlDestroyInstance"]
    mxl_destroy_instance:
        unsafe extern "C" fn(in_instance: mxl_sys::mxlInstance) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlCreateFlow"]
    mxl_create_flow: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        flowDef: *const std::os::raw::c_char,
        options: *const std::os::raw::c_char,
        info: *mut mxl_sys::FlowInfo,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlDestroyFlow"]
    mxl_destroy_flow: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        flowId: *const std::os::raw::c_char,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlCreateFlowReader"]
    mxl_create_flow_reader: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        flowId: *const std::os::raw::c_char,
        options: *const std::os::raw::c_char,
        reader: *mut mxl_sys::mxlFlowReader,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlReleaseFlowReader"]
    mxl_release_flow_reader: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        reader: mxl_sys::mxlFlowReader,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlCreateFlowWriter"]
    mxl_create_flow_writer: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        flowId: *const std::os::raw::c_char,
        options: *const std::os::raw::c_char,
        writer: *mut mxl_sys::mxlFlowWriter,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlReleaseFlowWriter"]
    mxl_release_flow_writer: unsafe extern "C" fn(
        instance: mxl_sys::mxlInstance,
        writer: mxl_sys::mxlFlowWriter,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowReaderGetInfo"]
    mxl_flow_reader_get_info: unsafe extern "C" fn(
        reader: mxl_sys::mxlFlowReader,
        info: *mut mxl_sys::FlowInfo,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlFlowReaderGetGrain"]
    mxl_flow_reader_get_grain: unsafe extern "C" fn(
        reader: mxl_sys::mxlFlowReader,
        index: u64,
        timeoutNs: u64,
        grain: *mut mxl_sys::GrainInfo,
        payload: *mut *mut u8,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowReaderGetGrainNonBlocking"]
    mxl_flow_reader_get_grain_non_blocking: unsafe extern "C" fn(
        reader: mxl_sys::mxlFlowReader,
        index: u64,
        grain: *mut mxl_sys::GrainInfo,
        payload: *mut *mut u8,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlFlowWriterOpenGrain"]
    mxl_flow_writer_open_grain: unsafe extern "C" fn(
        writer: mxl_sys::mxlFlowWriter,
        index: u64,
        grainInfo: *mut mxl_sys::GrainInfo,
        payload: *mut *mut u8,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowWriterCancelGrain"]
    mxl_flow_writer_cancel_grain:
        unsafe extern "C" fn(writer: mxl_sys::mxlFlowWriter) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowWriterCommitGrain"]
    mxl_flow_writer_commit_grain: unsafe extern "C" fn(
        writer: mxl_sys::mxlFlowWriter,
        grain: *const mxl_sys::GrainInfo,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlFlowReaderGetSamples"]
    mxl_flow_reader_get_samples: unsafe extern "C" fn(
        reader: mxl_sys::mxlFlowReader,
        index: u64,
        count: usize,
        payloadBuffersSlices: *mut mxl_sys::WrappedMultiBufferSlice,
    ) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlFlowWriterOpenSamples"]
    mxl_flow_writer_open_samples: unsafe extern "C" fn(
        writer: mxl_sys::mxlFlowWriter,
        index: u64,
        count: usize,
        payloadBuffersSlices: *mut mxl_sys::MutableWrappedMultiBufferSlice,
    ) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowWriterCancelSamples"]
    mxl_flow_writer_cancel_samples:
        unsafe extern "C" fn(writer: mxl_sys::mxlFlowWriter) -> mxl_sys::mxlStatus,
    #[dlopen2_name = "mxlFlowWriterCommitSamples"]
    mxl_flow_writer_commit_samples:
        unsafe extern "C" fn(writer: mxl_sys::mxlFlowWriter) -> mxl_sys::mxlStatus,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlGetCurrentHeadIndex"]
    mxl_get_current_head_index: unsafe extern "C" fn(editRate: *const mxl_sys::Rational) -> u64,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlGetNsUntilHeadIndex"]
    mxl_get_ns_until_head_index:
        unsafe extern "C" fn(index: u64, editRate: *const mxl_sys::Rational) -> u64,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlTimestampToHeadIndex"]
    mxl_timestamp_to_head_index:
        unsafe extern "C" fn(editRate: *const mxl_sys::Rational, timestamp: u64) -> u64,
    #[allow(non_snake_case)]
    #[dlopen2_name = "mxlHeadIndexToTimestamp"]
    mxl_head_index_to_timestamp:
        unsafe extern "C" fn(editRate: *const mxl_sys::Rational, index: u64) -> u64,
    #[dlopen2_name = "mxlSleepForNs"]
    mxl_sleep_for_ns: unsafe extern "C" fn(ns: u64),
    #[dlopen2_name = "mxlGetTime"]
    mxl_get_time: unsafe extern "C" fn() -> u64,
}

pub fn load_api(path_to_so_file: impl AsRef<Path>) -> Result<Container<MxlApi>, dlopen2::Error> {
    unsafe { Container::load(path_to_so_file.as_ref().as_os_str()) }
}

pub struct MxlInstance {
    api: Container<MxlApi>,
    instance: mxl_sys::mxlInstance,
}

impl MxlInstance {
    pub fn new(api: Container<MxlApi>, domain: &str, options: &str) -> Result<Self, MxlError> {
        let instance = unsafe {
            api.mxl_create_instance(
                CString::new(domain).unwrap().as_ptr(),
                CString::new(options).unwrap().as_ptr(),
            )
        };
        if instance.is_null() {
            Err(MxlError::Other(
                "Failed to create MXL instance.".to_string(),
            ))
        } else {
            Ok(Self { api, instance })
        }
    }

    pub fn destroy(&mut self) -> Result<(), MxlError> {
        let result;
        if self.instance.is_null() {
            return Err(MxlError::Other(
                "Internal instance not initialized.".to_string(),
            ));
        }
        unsafe {
            result = status_to_result(self.api.mxl_destroy_instance(self.instance));
        }
        self.instance = std::ptr::null_mut();
        result
    }

    pub fn create_flow_reader(&self, flow_id: &str) -> Result<MxlFlowReader, MxlError> {
        let flow_id = CString::new(flow_id).map_err(|_| MxlError::InvalidArg)?;
        let options = CString::new("").map_err(|_| MxlError::InvalidArg)?;
        let mut reader: mxl_sys::mxlFlowReader = std::ptr::null_mut();
        unsafe {
            status_to_result(self.api.mxl_create_flow_reader(
                self.instance,
                flow_id.as_ptr(),
                options.as_ptr(),
                &mut reader,
            ))?;
        }
        if reader.is_null() {
            return Err(MxlError::Other("Failed to create flow reader.".to_string()));
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
    pub fn discrete_flow_info(&self) -> Result<&mxl_sys::DiscreteFlowInfo, MxlError> {
        // Check is based on mxlIsDiscreteDataFormat, which is inline, thus not accessible in
        // mxl_sys.
        if self.value.common.format != mxl_sys::MXL_DATA_FORMAT_VIDEO
            && self.value.common.format != mxl_sys::MXL_DATA_FORMAT_DATA
        {
            return Err(MxlError::Other(format!(
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
    pub fn destroy(&mut self) -> Result<(), MxlError> {
        let result;
        if self.reader.is_null() {
            return Err(MxlError::InvalidArg);
        }
        unsafe {
            result = status_to_result(
                self.instance
                    .api
                    .mxl_release_flow_reader(self.instance.instance, self.reader),
            );
        }
        self.reader = std::ptr::null_mut();
        result
    }

    pub fn get_info(&self) -> Result<FlowInfo, MxlError> {
        let mut flow_info: mxl_sys::FlowInfo = unsafe { std::mem::zeroed() };
        unsafe {
            status_to_result(
                self.instance
                    .api
                    .mxl_flow_reader_get_info(self.reader, &mut flow_info),
            )?;
        }
        Ok(FlowInfo { value: flow_info })
    }

    pub fn get_complete_grain(&self, index: u64, timeout: Duration) -> Result<GrainData, MxlError> {
        let mut grain_info: mxl_sys::GrainInfo = unsafe { std::mem::zeroed() };
        let mut payload_ptr: *mut u8 = std::ptr::null_mut();
        let timeout_ns = timeout.as_nanos() as u64;
        loop {
            unsafe {
                status_to_result(self.instance.api.mxl_flow_reader_get_grain(
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
                return Err(MxlError::Other(format!(
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
