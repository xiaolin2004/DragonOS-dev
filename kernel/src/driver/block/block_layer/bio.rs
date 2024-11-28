use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Debug;
use core::ops::Range;
use core::sync::atomic::{AtomicU32, Ordering};
use system_error::SystemError;
use crate::driver::block::cache::BLOCK_SIZE;
use crate::libs::align_extend::lib::AlignExtend;
use crate::driver::base::block::block_device::GeneralBlockRange;
use crate::driver::base::device::Device;
use crate::libs::wait_queue::WaitQueue;
use crate::process::ProcessManager;
use crate::{driver::base::block::block_device::BlockDevice, process::ProcessControlBlock};

pub(crate) struct Bio(Arc<BioInner>);

impl Bio {
    /// Constructs a new `Bio`.
    ///
    /// The `type_` describes the type of the I/O.
    /// The `start_sid` is the starting sector id on the device.
    /// The `segments` describes the memory segments.
    /// The `complete_fn` is the optional callback function.
    pub fn new(
        type_: BioType,
        start_sid: usize,
        segments: Vec<BioSegment>,
        complete_fn: Option<fn(&SubmittedBio)>,
    ) -> Self {
        let nsectors = segments
            .iter()
            .map(|segment| segment.nsectors().to_raw())
            .sum();

        let inner = Arc::new(BioInner {
            type_,
            sid_range: start_sid..start_sid + nsectors,
            segments,
            complete_fn,
            status: AtomicU32::new(BioStatus::Init as u32),
            wait_queue: WaitQueue::default(),
        });
        Self(inner)
    }
    pub fn type_(&self) -> BioType {
        self.0.type_()
    }

    pub fn sid_range(&self) -> &GeneralBlockRange {
        self.0.sid_range()
    }

    /// Returns the slice to the memory segments.
    pub fn segments(&self) -> &[BioSegment] {
        self.0.segments()
    }

    /// Returns the status.
    pub fn status(&self) -> BioStatus {
        self.0.status()
    }

    pub fn submit(&self, block_device: &dyn BlockDevice) -> Result<BioWaiter, SystemError> {
        // Change the status from "Init" to "Submit".
        let result = self.0.status.compare_exchange(
            BioStatus::Init as u32,
            BioStatus::Submit as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        if let Err(e) = block_device.enqueue(SubmittedBio(self.0.clone())) {
            // Fail to submit, revert the status.
            let result = self.0.status.compare_exchange(
                BioStatus::Submit as u32,
                BioStatus::Init as u32,
                Ordering::Release,
                Ordering::Relaxed,
            );
            assert!(result.is_ok());
            return Err(e);
        }

        Ok(BioWaiter {
            bios: vec![self.0.clone()],
        })
    }

    pub fn submit_and_wait(
        &self,
        block_device: &dyn BlockDevice,
    ) -> Result<BioStatus, SystemError> {
        let waiter = self.submit(block_device)?;
        match waiter.wait() {
            Some(status) => {
                assert!(status == BioStatus::Complete);
                Ok(status)
            }
            None => {
                let status = self.status();
                assert!(status != BioStatus::Complete);
                Ok(status)
            }
        }
    }
}

/// The error type returned when enqueueing the `Bio`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BioEnqueueError {
    /// The request queue is full
    IsFull,
    /// Refuse to enqueue the bio
    Refused,
    /// Too big bio
    TooBig,
}

impl From<BioEnqueueError> for SystemError {
    fn from(error: BioEnqueueError) -> Self {
        match error {
            BioEnqueueError::IsFull => SystemError::EBUSY,
            BioEnqueueError::Refused => SystemError::EACCES,
            BioEnqueueError::TooBig => SystemError::E2BIG,
        }
    }
}

/// A waiter for `Bio` submissions.
///
/// This structure holds a list of `Bio` requests and provides functionality to
/// wait for their completion and retrieve their statuses.
#[derive(Debug)]
pub struct BioWaiter {
    bios: Vec<Arc<BioInner>>,
}

impl BioWaiter {
    /// Constructs a new `BioWaiter` instance with no `Bio` requests.
    pub fn new() -> Self {
        Self { bios: Vec::new() }
    }

    /// Returns the number of `Bio` requests associated with `self`.
    pub fn nreqs(&self) -> usize {
        self.bios.len()
    }

    /// Gets the `index`-th `Bio` request associated with `self`.
    ///
    /// # Panics
    ///
    /// If the `index` is out of bounds, this method will panic.
    pub fn req(&self, index: usize) -> Bio {
        Bio(self.bios[index].clone())
    }

    /// Returns the status of the `index`-th `Bio` request associated with `self`.
    ///
    /// # Panics
    ///
    /// If the `index` is out of bounds, this method will panic.
    pub fn status(&self, index: usize) -> BioStatus {
        self.bios[index].status()
    }

    /// Merges the `Bio` requests from another `BioWaiter` into this one.
    ///
    /// The another `BioWaiter`'s `Bio` requests are appended to the end of
    /// the `Bio` list of `self`, effectively concatenating the two lists.
    pub fn concat(&mut self, mut other: Self) {
        self.bios.append(&mut other.bios);
    }

    /// Waits for the completion of all `Bio` requests.
    ///
    /// This method iterates through each `Bio` in the list, waiting for their
    /// completion.
    ///
    /// The return value is an option indicating whether all the requests in the list
    /// have successfully completed.
    /// On success this value is guaranteed to be equal to `Some(BioStatus::Complete)`.
    pub fn wait(&self) -> Option<BioStatus> {
        let mut ret = Some(BioStatus::Complete);

        for bio in self.bios.iter() {
            let status = bio.wait_queue.wait_until(|| {
                let status = bio.status();
                if status != BioStatus::Submit {
                    Some(status)
                } else {
                    None
                }
            });
            if status != BioStatus::Complete && ret.is_some() {
                ret = None;
            }
        }

        ret
    }

    /// Clears all `Bio` requests in this waiter.
    pub fn clear(&mut self) {
        self.bios.clear();
    }
}

impl Default for BioWaiter {
    fn default() -> Self {
        Self::new()
    }
}

/// A submitted `Bio` object.
///
/// The request queue of block device only accepts a `SubmittedBio` into the queue.
#[derive(Debug)]
pub struct SubmittedBio(Arc<BioInner>);

impl SubmittedBio {
    /// Returns the type.
    pub fn type_(&self) -> BioType {
        self.0.type_()
    }

    /// Returns the range of target sectors on the device.
    pub fn sid_range(&self) -> &GeneralBlockRange {
        self.0.sid_range()
    }

    /// Returns the slice to the memory segments.
    pub fn segments(&self) -> &[BioSegment] {
        self.0.segments()
    }

    /// Returns the status.
    pub fn status(&self) -> BioStatus {
        self.0.status()
    }

    /// Completes the `Bio` with the `status` and invokes the callback function.
    ///
    /// When the driver finishes the request for this `Bio`, it will call this method.
    pub fn complete(&self, status: BioStatus) {
        assert!(status != BioStatus::Init && status != BioStatus::Submit);

        // Set the status.
        let result = self.0.status.compare_exchange(
            BioStatus::Submit as u32,
            status as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        self.0.wait_queue.wake_all();
        if let Some(complete_fn) = self.0.complete_fn {
            complete_fn(self);
        }
    }
}
struct BioInner {
    /// The type of the I/O
    type_: BioType,
    /// The range of the sector id on device
    sid_range: GeneralBlockRange,
    /// The memory segments in this `Bio`
    segments: Vec<BioSegment>,
    /// The I/O completion method
    complete_fn: Option<fn(&SubmittedBio)>,
    /// The I/O status
    status: AtomicU32,
    /// The wait queue for I/O completion
    wait_queue: WaitQueue,
}

impl BioInner {
    pub fn type_(&self) -> BioType {
        self.type_
    }

    pub fn sid_range(&self) -> &GeneralBlockRange {
        &self.sid_range
    }

    pub fn segments(&self) -> &[BioSegment] {
        &self.segments
    }

    pub fn status(&self) -> BioStatus {
        BioStatus::try_from(self.status.load(Ordering::Relaxed)).unwrap()
    }
}

impl Debug for BioInner {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BioInner")
            .field("type", &self.type_())
            .field("sid_range", &self.sid_range())
            .field("status", &self.status())
            .field("segments", &self.segments())
            .field("complete_fn", &self.complete_fn)
            .finish()
    }
}
/// The type of `Bio`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum BioType {
    /// Read sectors from the device.
    Read = 0,
    /// Write sectors into the device.
    Write = 1,
    /// Flush the volatile write cache.
    Flush = 2,
    /// Discard sectors.
    Discard = 3,
}

/// The status of `Bio`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum BioStatus {
    /// The initial status for a newly created `Bio`.
    Init = 0,
    /// After a `Bio` is submitted, its status will be changed to "Submit".
    Submit = 1,
    /// The I/O operation has been successfully completed.
    Complete = 2,
    /// The I/O operation is not supported.
    NotSupported = 3,
    /// Insufficient space is available to perform the I/O operation.
    NoSpace = 4,
    /// An error occurred while doing I/O.
    IoError = 5,
}

impl TryFrom<u32> for BioStatus {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(BioStatus::Init),
            1 => Ok(BioStatus::Submit),
            2 => Ok(BioStatus::Complete),
            3 => Ok(BioStatus::NotSupported),
            4 => Ok(BioStatus::NoSpace),
            5 => Ok(BioStatus::IoError),
            _ => Err(()),
        }
    }
}

/// `BioSegment` is a smallest memory unit in block I/O.
///
/// It is a contiguous memory region that contains multiple sectors.
#[derive(Debug, Clone)]
pub struct BioSegment {
    /// The contiguous pages on which this segment resides.
    pages: Segment,
    /// The starting offset (in bytes) within the first page.
    /// The offset should always be aligned to the sector size and
    /// must not exceed the size of a single page.
    offset: AlignedUsize<SECTOR_SIZE>,
    /// The total length (in bytes).
    /// The length can span multiple pages and should be aligned to
    /// the sector size.
    len: AlignedUsize<SECTOR_SIZE>,
}

const SECTOR_SIZE: u16 = 512;

impl<'a> BioSegment {
    /// Constructs a new `BioSegment` from `Segment`.
    pub fn from_segment(segment: Segment, offset: usize, len: usize) -> Self {
        assert!(offset + len <= segment.nbytes());

        Self {
            pages: segment.range(frame_range(&(offset..offset + len))),
            offset: AlignedUsize::<SECTOR_SIZE>::new(offset % BLOCK_SIZE).unwrap(),
            len: AlignedUsize::<SECTOR_SIZE>::new(len).unwrap(),
        }
    }

    /// Constructs a new `BioSegment` from `Frame`.
    pub fn from_frame(frame: Frame, offset: usize, len: usize) -> Self {
        assert!(offset + len <= super::BLOCK_SIZE);

        Self {
            pages: Segment::from(frame),
            offset: AlignedUsize::<SECTOR_SIZE>::new(offset).unwrap(),
            len: AlignedUsize::<SECTOR_SIZE>::new(len).unwrap(),
        }
    }

    /// Returns the number of sectors.
    pub fn nsectors(&self) -> Sid {
        Sid::from_offset(self.len.value())
    }

    /// Returns the number of bytes.
    pub fn nbytes(&self) -> usize {
        self.len.value()
    }

    /// Returns the offset (in bytes) within the first page.
    pub fn offset(&self) -> usize {
        self.offset.value()
    }

    /// Returns the contiguous pages on which this segment resides.
    pub fn pages(&self) -> &Segment {
        &self.pages
    }

    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a, Infallible> {
        self.pages
            .reader()
            .skip(self.offset.value())
            .limit(self.len.value())
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a, Infallible> {
        self.pages
            .writer()
            .skip(self.offset.value())
            .limit(self.len.value())
    }
}

fn frame_range(byte_range: &Range<usize>) -> Range<usize> {
    let start = byte_range.start.align_down(BLOCK_SIZE);
    let end = byte_range.end.align_up(BLOCK_SIZE);
    (start /BLOCK_SIZE)..(end / BLOCK_SIZE)
}

#[derive(Debug, Clone, Copy)]
pub struct AlignedUsize<const N: u16>(usize);

impl<const N: u16> AlignedUsize<N>{
    pub fn new(value: usize) -> Option<Self>{
        if value % (N as usize) ==0{
            Some(Self(value))
        }else{
            None
        }
    }
    
    pub fn value(&self) -> usize{
        self.0
    }   

    /// 用于计算block，sector，page的id
    pub fn id(self) -> usize{
        self.0 / (N as usize)
    }

    pub fn align(&self) -> usize{
        N as usize
    }
}