//! Windows named pipes for the project endpoint: the pipe kerfd listens on,
//! open only to the user it runs as, and the client end, each with read and
//! write timeouts like a Unix socket's.
//!
//! Every handle is opened for overlapped I/O so a read, a write, or a wait
//! for a client can give up after a timeout; each operation still finishes,
//! or is cancelled and waited out, before its call returns.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TokenUser, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// One connected end of a pipe: a client's, or kerfd's for one client.
pub(crate) struct PipeStream {
    pipe: OwnedHandle,
    /// Signalled when the pending operation on `pipe` completes.
    event: OwnedHandle,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl PipeStream {
    fn new(pipe: OwnedHandle) -> io::Result<Self> {
        Ok(Self {
            pipe,
            event: manual_reset_event()?,
            read_timeout: None,
            write_timeout: None,
        })
    }

    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.read_timeout = timeout;
        Ok(())
    }

    pub(crate) fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.write_timeout = timeout;
        Ok(())
    }

    /// Wait until the other end has read everything written to it, or has
    /// gone. Closing a server end straight after writing could otherwise
    /// lose the response.
    pub(crate) fn wait_until_read(&self) {
        // SAFETY: the handle is owned by `self` and open.
        unsafe { FlushFileBuffers(self.handle()) };
    }

    fn handle(&self) -> HANDLE {
        self.pipe.as_raw_handle() as HANDLE
    }

    /// Start an operation with `start` and wait up to `timeout` for it,
    /// returning the bytes it transferred. One that runs out of time is
    /// cancelled and reported as `TimedOut`.
    fn complete(
        &self,
        timeout: Option<Duration>,
        start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32,
    ) -> io::Result<u32> {
        // SAFETY: an all-zero OVERLAPPED is its documented initial state.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.event.as_raw_handle() as HANDLE;
        let handle = self.handle();
        if start(handle, &mut overlapped) == 0 {
            // SAFETY: reads the calling thread's last error.
            let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if error != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            // SAFETY: the event handle is open for as long as `self`.
            if unsafe { WaitForSingleObject(overlapped.hEvent, wait_millis(timeout)) }
                == WAIT_TIMEOUT
            {
                // SAFETY: `overlapped` names the pending operation on
                // `handle`; the wait below lets it finish before it goes.
                unsafe { CancelIoEx(handle, &overlapped) };
            }
        }
        let mut transferred = 0;
        // SAFETY: waits for the operation `overlapped` describes, which
        // lives until after this call, to finish or be cancelled.
        if unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, 1) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "named pipe operation timed out",
                ));
            }
            return Err(error);
        }
        Ok(transferred)
    }
}

impl Read for PipeStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let pointer = buffer.as_mut_ptr();
        match self.complete(self.read_timeout, |handle, overlapped| {
            // SAFETY: `buffer` outlives the operation, which `complete`
            // waits out before returning.
            unsafe { ReadFile(handle, pointer, length, ptr::null_mut(), overlapped) }
        }) {
            Ok(count) => Ok(count as usize),
            // The other end closed: the end of the stream, as for a socket.
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            Err(error) => Err(error),
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let pointer = buffer.as_ptr();
        self.complete(self.write_timeout, |handle, overlapped| {
            // SAFETY: as for the read above.
            unsafe { WriteFile(handle, pointer, length, ptr::null_mut(), overlapped) }
        })
        .map(|count| count as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Connect to the pipe `name`, waiting up to `timeout` while every instance
/// is busy. The pipe must belong to the user this process runs as: one that
/// another user created first under the same name is refused with
/// `PermissionDenied`, before anything is written to it.
pub(crate) fn connect(name: &str, timeout: Duration) -> io::Result<PipeStream> {
    let wide = wide(name);
    let deadline = Instant::now() + timeout;
    let pipe = loop {
        // SAFETY: `wide` is a NUL-terminated name that outlives the call.
        // The server may identify this client but not act as it.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: a valid handle this function now owns.
            break unsafe { OwnedHandle::from_raw_handle(handle as _) };
        }
        let error = io::Error::last_os_error();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) || remaining.is_zero() {
            return Err(error);
        }
        // Every instance is serving a client; kerfd makes another as soon
        // as one connects.
        // SAFETY: as for CreateFileW above.
        unsafe { WaitNamedPipeW(wide.as_ptr(), wait_millis(Some(remaining)).max(1)) };
    };
    let user = UserSid::current()?;
    if !user.owns(pipe.as_raw_handle() as HANDLE)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{name} belongs to another user"),
        ));
    }
    PipeStream::new(pipe)
}

/// The server end: one instance waiting for a client at a time, each made
/// for the current user only.
pub(crate) struct PipeListener {
    name: Vec<u16>,
    security: SecurityDescriptor,
    instance: PipeStream,
    /// The pending connect on `instance`, boxed so it stays put while the
    /// kernel holds it.
    overlapped: Box<OVERLAPPED>,
    connecting: bool,
}

impl PipeListener {
    /// Create the pipe `name`. Fails with `AddrInUse` when the name is
    /// taken, by another kerfd or anything else.
    pub(crate) fn bind(name: &str) -> io::Result<Self> {
        let name = wide(name);
        let security = SecurityDescriptor::current_user_only()?;
        let instance = create_instance(&name, &security, true).map_err(|error| {
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                io::Error::new(io::ErrorKind::AddrInUse, error)
            } else {
                error
            }
        })?;
        Ok(Self {
            name,
            security,
            instance,
            // SAFETY: an all-zero OVERLAPPED is its documented initial state.
            overlapped: Box::new(unsafe { std::mem::zeroed() }),
            connecting: false,
        })
    }

    /// Wait up to `timeout` for a client. Returns its connection, or `None`
    /// when none came.
    pub(crate) fn accept(&mut self, timeout: Duration) -> io::Result<Option<PipeStream>> {
        let handle = self.instance.handle();
        if !self.connecting {
            // SAFETY: no operation holds the OVERLAPPED while not connecting.
            *self.overlapped = unsafe { std::mem::zeroed() };
            self.overlapped.hEvent = self.instance.event.as_raw_handle() as HANDLE;
            // SAFETY: the boxed OVERLAPPED stays in place until the connect
            // completes or `drop` waits out its cancellation.
            if unsafe { ConnectNamedPipe(handle, &mut *self.overlapped) } == 0 {
                let error = io::Error::last_os_error();
                match error.raw_os_error().map(|code| code as u32) {
                    Some(ERROR_IO_PENDING) => self.connecting = true,
                    // A client arrived between creating and connecting.
                    Some(ERROR_PIPE_CONNECTED) => return self.next_instance().map(Some),
                    // A client came and went already.
                    Some(ERROR_NO_DATA) => {
                        self.next_instance()?;
                        return Ok(None);
                    }
                    _ => return Err(error),
                }
            } else {
                return self.next_instance().map(Some);
            }
        }
        // SAFETY: the event handle is open for as long as the instance.
        match unsafe { WaitForSingleObject(self.overlapped.hEvent, wait_millis(Some(timeout))) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                self.connecting = false;
                let mut transferred = 0;
                // SAFETY: the connect has completed; this only reads its
                // result.
                let connected =
                    unsafe { GetOverlappedResult(handle, &*self.overlapped, &mut transferred, 0) }
                        != 0;
                if connected {
                    return self.next_instance().map(Some);
                }
                let error = io::Error::last_os_error();
                match error.raw_os_error().map(|code| code as u32) {
                    Some(ERROR_NO_DATA | ERROR_BROKEN_PIPE) => {
                        self.next_instance()?;
                        Ok(None)
                    }
                    _ => Err(error),
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Hand back the instance a client connected to, leaving a fresh one to
    /// wait for the next.
    fn next_instance(&mut self) -> io::Result<PipeStream> {
        let next = create_instance(&self.name, &self.security, false)?;
        Ok(std::mem::replace(&mut self.instance, next))
    }
}

impl Drop for PipeListener {
    fn drop(&mut self) {
        if self.connecting {
            let handle = self.instance.handle();
            let mut transferred = 0;
            // SAFETY: cancel the pending connect and wait for the kernel to
            // let go of the OVERLAPPED before it is freed.
            unsafe {
                CancelIoEx(handle, &*self.overlapped);
                GetOverlappedResult(handle, &*self.overlapped, &mut transferred, 1);
            }
        }
    }
}

fn create_instance(
    name: &[u16],
    security: &SecurityDescriptor,
    first: bool,
) -> io::Result<PipeStream> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.0,
        bInheritHandle: 0,
    };
    let first = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    // SAFETY: the name and the security attributes outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | first,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle this function now owns.
    PipeStream::new(unsafe { OwnedHandle::from_raw_handle(handle as _) })
}

/// A security descriptor allocated by Windows, freed with `LocalFree`.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is immutable once made and owned by this value.
unsafe impl Send for SecurityDescriptor {}

impl SecurityDescriptor {
    /// Owned by the current user, who alone may open the pipe: no other
    /// user, and no inherited entries.
    fn current_user_only() -> io::Result<Self> {
        let sid = UserSid::current()?.string()?;
        let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; Windows allocates the result.
        let made = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if made == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by Windows with LocalAlloc, and freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// The user this process runs as.
struct UserSid {
    /// A TOKEN_USER whose SID points into this buffer, kept 8-byte aligned.
    buffer: Vec<u64>,
}

impl UserSid {
    fn current() -> io::Result<Self> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the current-process pseudo handle needs no closing; the
        // token handle is owned below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a valid handle this function now owns.
        let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
        let mut length = 0;
        // SAFETY: asks only for the size the information needs.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenUser,
                ptr::null_mut(),
                0,
                &mut length,
            )
        };
        let mut buffer = vec![0_u64; (length as usize).div_ceil(8).max(1)];
        // SAFETY: the buffer holds at least `length` bytes.
        let read = unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { buffer })
    }

    fn sid(&self) -> PSID {
        // SAFETY: the buffer holds the TOKEN_USER GetTokenInformation wrote.
        unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    /// The SID in its `S-1-5-…` form.
    fn string(&self) -> io::Result<String> {
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: the SID is valid; Windows allocates the string.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a NUL-terminated string Windows allocated, read and then
        // freed once.
        let string = unsafe {
            let length = (0..).take_while(|&index| *text.add(index) != 0).count();
            let string = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            LocalFree(text.cast());
            string
        };
        Ok(string)
    }

    /// Whether this user owns the kernel object `handle` names.
    fn owns(&self, handle: HANDLE) -> io::Result<bool> {
        let mut owner: PSID = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `owner` points into `descriptor`, which Windows allocates
        // and which is freed after the comparison.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        // SAFETY: both SIDs are valid; the descriptor is freed once.
        let same = unsafe {
            let same = EqualSid(owner, self.sid()) != 0;
            LocalFree(descriptor);
            same
        };
        Ok(same)
    }
}

fn manual_reset_event() -> io::Result<OwnedHandle> {
    // SAFETY: an unnamed, unsignalled, manual-reset event.
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle this function now owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(event as _) })
}

fn wait_millis(timeout: Option<Duration>) -> u32 {
    timeout.map_or(INFINITE, |timeout| {
        u32::try_from(timeout.as_millis())
            .unwrap_or(INFINITE - 1)
            .min(INFINITE - 1)
    })
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
