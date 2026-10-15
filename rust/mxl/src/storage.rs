// SPDX-FileCopyrightText: 2026 Contributors to the Media eXchange Layer project.
// SPDX-License-Identifier: Apache-2.0

//! Storage of the grain payloads of discrete flows.
//!
//! The payload of a discrete flow is in host memory, or in storage that a payload backend holds,
//! such as device memory. The layout describes the storage of all the slots of a flow, and each
//! slot has its own description. Storage types other than host memory keep their own fields, such
//! as handles, in the reserved bytes of both, which `reserved` and `reserved_mut` give access to.

use std::sync::Arc;

use crate::{reader::FlowReaderResource, writer::FlowWriterResource};

/// The payload is in host memory, and each slot is described by a host pointer.
pub const PAYLOAD_STORAGE_HOST_POINTER: u32 = mxl_sys::MXL_PAYLOAD_STORAGE_HOST_POINTER;

/// The payload is in CUDA device memory, and each slot is described by a device pointer.
pub const PAYLOAD_STORAGE_CUDA_DEVICE_POINTER: u32 =
    mxl_sys::MXL_PAYLOAD_STORAGE_CUDA_DEVICE_POINTER;

/// The number of valid slices to wait for to get a complete grain.
pub const GRAIN_VALID_SLICES_ALL: u16 = u16::MAX;

/// Version of the storage structures that this crate fills.
const STORAGE_STRUCT_VERSION: u32 = 1;

/// Position and geometry of one plane of a grain payload.
pub type GrainPlaneLayout = mxl_sys::GrainPlaneLayout;

/// Describes the storage of the payload of all the slots of a discrete flow.
///
/// The grain at index `i` is in slot `i % slot_count`. The layout does not change for the
/// lifetime of a flow.
#[derive(Clone)]
#[repr(transparent)]
pub struct GrainStorageLayout {
    pub(crate) value: mxl_sys::GrainStorageLayout,
}

impl GrainStorageLayout {
    pub(crate) fn zeroed() -> Self {
        Self {
            // SAFETY: the structure only holds integers and arrays of integers.
            value: unsafe { std::mem::zeroed() },
        }
    }

    pub fn storage_type(&self) -> u32 {
        self.value.storageType
    }

    pub fn is_host_storage(&self) -> bool {
        self.value.storageType == PAYLOAD_STORAGE_HOST_POINTER
    }

    /// Index, in this process, of the device that holds the payload, or `None` if the payload is
    /// not tied to a device. Indices can differ between processes. Use `device_uuid` to identify a
    /// device across processes.
    pub fn device_index(&self) -> Option<i32> {
        (self.value.deviceIndex >= 0).then_some(self.value.deviceIndex)
    }

    /// UUID of the device that holds the payload, all zeros if the payload is not tied to one.
    pub fn device_uuid(&self) -> [u8; 16] {
        self.value.deviceUuid
    }

    pub fn set_device_uuid(&mut self, uuid: [u8; 16]) {
        self.value.deviceUuid = uuid;
    }

    /// The number of slots in the ring buffer, equal to the grain count of the flow.
    pub fn slot_count(&self) -> u32 {
        self.value.slotCount
    }

    /// The planes of a grain. Video v210 has 1, v210a has 2: fill, then key.
    pub fn planes(&self) -> &[GrainPlaneLayout] {
        let count = (self.value.planeCount as usize).min(self.value.planes.len());
        &self.value.planes[..count]
    }

    /// The planes of a grain, for a writer that provides its storage. It may give planes a larger
    /// pitch, other offsets or a larger size than the requirements.
    pub fn planes_mut(&mut self) -> &mut [GrainPlaneLayout] {
        let count = (self.value.planeCount as usize).min(self.value.planes.len());
        &mut self.value.planes[..count]
    }

    /// The bytes that describe a storage type other than host memory. Zero for host storage.
    pub fn reserved(&self) -> &[u8] {
        &self.value.reserved
    }

    pub fn reserved_mut(&mut self) -> &mut [u8] {
        &mut self.value.reserved
    }

    pub fn as_raw(&self) -> &mxl_sys::GrainStorageLayout {
        &self.value
    }

    pub fn from_raw(value: mxl_sys::GrainStorageLayout) -> Self {
        Self { value }
    }
}

impl std::fmt::Debug for GrainStorageLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrainStorageLayout")
            .field("storage_type", &self.storage_type())
            .field("device_index", &self.device_index())
            .field("device_uuid", &self.device_uuid())
            .field("slot_count", &self.slot_count())
            .field("planes", &self.planes())
            .finish_non_exhaustive()
    }
}

/// Describes the storage of the payload of one slot of a discrete flow.
///
/// Handles in a description that the library filled, such as file descriptors, belong to the
/// library. They stay valid as long as the `MappedSlots` that holds the description.
#[derive(Clone)]
#[repr(transparent)]
pub struct GrainStorage {
    pub(crate) value: mxl_sys::GrainStorage,
}

impl GrainStorage {
    /// A description of a slot of storage with the given layout. The fields of the storage type go
    /// in `reserved_mut`.
    pub fn new(layout: &GrainStorageLayout, slot: u32) -> Self {
        let mut result = Self::zeroed();
        result.value.version = STORAGE_STRUCT_VERSION;
        result.value.size = std::mem::size_of::<mxl_sys::GrainStorage>() as u32;
        result.value.storageType = layout.storage_type();
        result.value.slot = slot;
        result
    }

    pub(crate) fn zeroed() -> Self {
        Self {
            // SAFETY: the structure only holds integers, arrays of integers and a pointer, for
            // which zero is a valid value.
            value: unsafe { std::mem::zeroed() },
        }
    }

    pub fn storage_type(&self) -> u32 {
        self.value.storageType
    }

    /// The slot that this structure describes.
    pub fn slot(&self) -> u32 {
        self.value.slot
    }

    /// The address of the first byte of the slot, if the payload is in host memory. The planes
    /// are at the offsets of the layout.
    pub fn host_pointer(&self) -> Option<*mut u8> {
        (self.value.storageType == PAYLOAD_STORAGE_HOST_POINTER)
            // SAFETY: the host member is the valid one for host storage.
            .then(|| unsafe { self.value.__bindgen_anon_1.host.pointer.cast::<u8>() })
    }

    /// The CUDA device address of the first byte of the slot, valid in this process on the device
    /// of the layout, if the payload is in CUDA device memory. The planes are at the offsets of the
    /// layout. Device work that writes a grain must be complete before the grain is committed.
    pub fn cuda_device_pointer(&self) -> Option<*mut std::ffi::c_void> {
        // SAFETY: every bit pattern of the union is a valid pointer, and it is only returned for
        // CUDA device storage, for which the cuda member is the valid one.
        let pointer = unsafe { self.value.__bindgen_anon_1.cuda.pointer };
        (self.value.storageType == PAYLOAD_STORAGE_CUDA_DEVICE_POINTER).then_some(pointer)
    }

    /// The bytes that describe the slot for a storage type other than host memory.
    pub fn reserved(&self) -> &[u8] {
        // SAFETY: every bit pattern of the union is a valid byte array.
        unsafe { &self.value.__bindgen_anon_1.reserved }
    }

    pub fn reserved_mut(&mut self) -> &mut [u8] {
        // SAFETY: every bit pattern of the union is a valid byte array.
        unsafe { &mut self.value.__bindgen_anon_1.reserved }
    }

    pub fn as_raw(&self) -> &mxl_sys::GrainStorage {
        &self.value
    }

    pub fn from_raw(value: mxl_sys::GrainStorage) -> Self {
        Self { value }
    }
}

impl std::fmt::Debug for GrainStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrainStorage")
            .field("storage_type", &self.storage_type())
            .field("slot", &self.slot())
            .finish_non_exhaustive()
    }
}

/// Keeps the reader or the writer whose library owns the handles in mapped slot descriptions.
enum SlotOwner {
    Reader(#[allow(dead_code)] Arc<FlowReaderResource>),
    Writer(#[allow(dead_code)] Arc<FlowWriterResource>),
}

/// The storage of every slot of a flow, mapped into this process.
///
/// It keeps the reader or writer that mapped the slots alive, so the handles in the descriptions,
/// which the library owns, stay valid as long as this object. An application that keeps a handle
/// longer duplicates it.
pub struct MappedSlots {
    slots: Vec<GrainStorage>,
    _owner: SlotOwner,
}

impl MappedSlots {
    pub(crate) fn from_reader(slots: Vec<GrainStorage>, reader: Arc<FlowReaderResource>) -> Self {
        Self {
            slots,
            _owner: SlotOwner::Reader(reader),
        }
    }

    pub(crate) fn from_writer(slots: Vec<GrainStorage>, writer: Arc<FlowWriterResource>) -> Self {
        Self {
            slots,
            _owner: SlotOwner::Writer(writer),
        }
    }

    /// The description of every slot, indexed by slot.
    pub fn slots(&self) -> &[GrainStorage] {
        &self.slots
    }

    pub fn get(&self, slot: u32) -> Option<&GrainStorage> {
        self.slots.get(slot as usize)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// A grain that a reader found in a slot, without access to its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrainSlot {
    /// The slot that holds the grain, an index into `MappedSlots`.
    pub slot: u32,
    /// The index of the grain in the slot. A different index than the one requested means that
    /// the requested grain was not written yet, if lower, or was overwritten, if higher.
    pub index: u64,
    pub flags: u32,
    /// Size in bytes of the payload of a complete grain.
    pub grain_size: u32,
    pub total_slices: u16,
    pub valid_slices: u16,
}

impl GrainSlot {
    pub(crate) fn new(info: &mxl_sys::GrainInfo, slot: u32) -> Self {
        Self {
            slot,
            index: info.index,
            flags: info.flags,
            grain_size: info.grainSize,
            total_slices: info.totalSlices,
            valid_slices: info.validSlices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_pointers_follow_the_storage_type() {
        let mut layout = GrainStorageLayout::zeroed();
        layout.value.storageType = PAYLOAD_STORAGE_CUDA_DEVICE_POINTER;
        let mut storage = GrainStorage::new(&layout, 3);
        let address = 0x7f00_0000_1000usize as *mut std::ffi::c_void;
        storage.value.__bindgen_anon_1.cuda.pointer = address;
        assert_eq!(storage.slot(), 3);
        assert_eq!(storage.cuda_device_pointer(), Some(address));
        assert_eq!(storage.host_pointer(), None);

        layout.value.storageType = PAYLOAD_STORAGE_HOST_POINTER;
        let storage = GrainStorage::new(&layout, 0);
        assert!(storage.host_pointer().is_some());
        assert_eq!(storage.cuda_device_pointer(), None);
    }
}
