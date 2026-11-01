//! Windows job objects: a sidecar and whatever it starts end together, when
//! it is cancelled and when the process that ran it ends, however it ends.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;
use std::ptr;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// A job holding one sidecar. Everything in it is killed once its last
/// handle closes: when this value is dropped, or when the process holding
/// it ends, so sidecars do not outlive kerfd.
pub(super) struct Job(OwnedHandle);

impl Job {
    /// Put `child` in a job of its own. What it starts joins the job too;
    /// only a process it starts before this call returns could escape it.
    pub(super) fn assign(child: &Child) -> io::Result<Self> {
        // SAFETY: an unnamed job with default security; its handle is not
        // inherited by children.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a valid handle this function now owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle as _) });
        // SAFETY: an all-zero limit structure sets no limits.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is the structure the information class names,
        // and outlives the call.
        let limited = unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if limited == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both handles are open; `child` is not yet reaped.
        if unsafe { AssignProcessToJobObject(job.handle(), child.as_raw_handle() as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Kill every process in the job.
    pub(super) fn terminate(&self) {
        // SAFETY: the job handle is open.
        unsafe { TerminateJobObject(self.handle(), 1) };
    }

    fn handle(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }
}
