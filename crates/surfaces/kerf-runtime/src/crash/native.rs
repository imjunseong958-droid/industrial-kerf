//! What a crash that is not a panic leaves behind: a bad memory access or an abort in C or C++,
//! such as the editor's own code, Qt or FFmpeg, where Rust's panic hook never runs. A signal
//! handler (macOS, Linux), or an unhandled-exception filter and the C runtime's `SIGABRT`
//! (Windows), writes a report into the crash folder in a panic's format, so `reports` lists it and
//! the editor tells of it the next time it opens.
//!
//! The handler runs in a process in an unknown state, perhaps holding the allocator's lock: it
//! allocates nothing and takes no lock. It uses what is safe in a signal handler (`mkdir`,
//! `open`, `write`, `close`, `clock_gettime`, `sigaction`, `raise`; on Windows the file, time and
//! module calls), on text made when it was installed and buffers on its own stack. The backtrace
//! is glibc's `backtrace`, which unwinds through the signal's frame and is loaded once at install
//! so the handler does not load it; on macOS the frame pointers of the interrupted code, named by
//! `backtrace_symbols_fd`; and on Windows `RtlCaptureStackBackTrace`, each frame named by its
//! module and offset. After writing, the signal goes on as it would have without the handler, so
//! the process still dies of it and the system's own crash reporting still sees it.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

/// Set by the first report: a second crash, or the same one on another thread, writes none.
static REPORTED: AtomicBool = AtomicBool::new(false);

/// When a panic's report was last written. An abort that follows within [`PANIC_THEN_ABORT_MS`]
/// is that panic ending the process (one that cannot unwind out of an `extern "C"` function), and
/// the panic's report already says more than the abort's would.
static LAST_PANIC_MS: AtomicU64 = AtomicU64::new(0);
const PANIC_THEN_ABORT_MS: u64 = 5_000;

/// What the handler needs, made when it is installed.
struct Setup {
    /// The folder, without a separator at its end.
    folder: Folder,
    /// What a report's file name starts with: `<component>-`.
    component: String,
    /// The first lines of a report, through `os:`, as `report_text` writes them.
    header: String,
    /// The thread that installed it, the main thread of kerfd, kerf and the editor.
    main_thread: u64,
    #[cfg(unix)]
    unix: unix::Previous,
}

#[cfg(unix)]
type Folder = std::ffi::CString;
#[cfg(windows)]
type Folder = Vec<u16>;
#[cfg(not(any(unix, windows)))]
type Folder = ();

static SETUP: OnceLock<Setup> = OnceLock::new();

/// Install the handlers for `component`, writing into `folder`. Only the first call in a process
/// takes effect.
pub(super) fn install(component: &str, folder: &Path) {
    if SETUP.get().is_some() {
        return;
    }
    let Some(folder) = folder_for_handler(folder) else {
        return;
    };
    let header = format!(
        "Kerf crash report\ncomponent: {component}\nversion: {}\nos: {} {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    #[cfg(unix)]
    let setup = Setup {
        folder,
        component: component.to_owned(),
        header,
        main_thread: unix::thread_id(),
        unix: unix::Previous::read(),
    };
    #[cfg(windows)]
    let setup = Setup { folder, component: component.to_owned(), header, main_thread: windows::thread_id() };
    #[cfg(not(any(unix, windows)))]
    let setup = Setup { folder, component: component.to_owned(), header, main_thread: 0 };
    if SETUP.set(setup).is_err() {
        return;
    }
    #[cfg(unix)]
    unix::install();
    #[cfg(windows)]
    windows::install();
}

#[cfg(unix)]
fn folder_for_handler(folder: &Path) -> Option<Folder> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = folder.as_os_str().as_bytes();
    let trimmed = match bytes {
        [rest @ .., b'/'] if !rest.is_empty() => rest,
        bytes => bytes,
    };
    std::ffi::CString::new(trimmed).ok()
}

#[cfg(windows)]
fn folder_for_handler(folder: &Path) -> Option<Folder> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = folder.as_os_str().encode_wide().collect();
    while wide.len() > 3 && matches!(wide.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        wide.pop();
    }
    (!wide.is_empty() && !wide.contains(&0)).then_some(wide)
}

#[cfg(not(any(unix, windows)))]
fn folder_for_handler(_: &Path) -> Option<Folder> {
    None
}

/// The panic hook says it wrote a report.
pub(super) fn panic_reported() {
    LAST_PANIC_MS.store(now_ms(), Ordering::SeqCst);
}

/// Report `message` now, as a fatal error the process is about to die of, such as Qt's `qFatal`
/// before its abort; the crash that follows writes no second report. Not from a signal handler:
/// this one may allocate. Nothing is written when no handler was installed.
pub(super) fn fatal(message: &str) -> Option<std::path::PathBuf> {
    let setup = SETUP.get()?;
    if REPORTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    let at_ms = now_ms();
    let thread = std::thread::current();
    let thread = thread.name().map_or_else(|| format!("{}", current_thread()), str::to_owned);
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let text = format!(
        "{}at_ms: {at_ms}\nthread: {thread}\nlocation: unknown\nmessage: {}\n\n{message}\n\nbacktrace:\n{backtrace}\n",
        setup.header,
        message.lines().next().unwrap_or(""),
    );
    let folder = folder_path(setup);
    std::fs::create_dir_all(&folder).ok()?;
    let path = folder.join(format!("{}-{at_ms}-{}.txt", setup.component, std::process::id()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(unix)]
fn folder_path(setup: &Setup) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(setup.folder.as_bytes()))
}

#[cfg(windows)]
fn folder_path(setup: &Setup) -> std::path::PathBuf {
    use std::os::windows::ffi::OsStringExt;
    std::path::PathBuf::from(std::ffi::OsString::from_wide(&setup.folder))
}

#[cfg(not(any(unix, windows)))]
fn folder_path(_: &Setup) -> std::path::PathBuf {
    std::path::PathBuf::new()
}

fn current_thread() -> u64 {
    #[cfg(unix)]
    return unix::thread_id();
    #[cfg(windows)]
    return windows::thread_id();
    #[cfg(not(any(unix, windows)))]
    0
}

/// Whether a report should be written now, for a crash of this kind: once per process, and an
/// abort right after a panic's report is that panic's.
fn first_report(abort: bool) -> bool {
    let after_panic = abort && now_ms().saturating_sub(LAST_PANIC_MS.load(Ordering::SeqCst)) < PANIC_THEN_ABORT_MS;
    !after_panic && !REPORTED.swap(true, Ordering::SeqCst)
}

/// Milliseconds since 1970, from a call that is safe in a signal handler.
fn now_ms() -> u64 {
    #[cfg(unix)]
    {
        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: clock_gettime writes the timespec it is given, and is async-signal-safe.
        unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut now) };
        (now.tv_sec as u64) * 1000 + (now.tv_nsec as u64) / 1_000_000
    }
    #[cfg(windows)]
    {
        windows::now_ms()
    }
    #[cfg(not(any(unix, windows)))]
    {
        0
    }
}

/// Text made without allocating, in a buffer on the handler's stack; what does not fit is left
/// out.
struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Text<N> {
    fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }

    fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        let take = bytes.len().min(N - self.len);
        self.bytes[self.len..self.len + take].copy_from_slice(&bytes[..take]);
        self.len += take;
        self
    }

    fn str(&mut self, text: &str) -> &mut Self {
        self.bytes(text.as_bytes())
    }

    fn dec(&mut self, mut value: u64) -> &mut Self {
        let mut digits = [0u8; 20];
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.bytes(&digits[at..])
    }

    fn hex(&mut self, mut value: u64) -> &mut Self {
        let mut digits = [0u8; 16];
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b"0123456789abcdef"[(value & 0xf) as usize];
            value >>= 4;
            if value == 0 {
                break;
            }
        }
        self.str("0x").bytes(&digits[at..])
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Whether it all fit.
    fn whole(&self) -> bool {
        self.len < N
    }
}

/// The lines of a report after `os:`, through `backtrace:`, as `report_text` writes them.
fn report_head<const N: usize>(
    text: &mut Text<N>,
    setup: &Setup,
    at_ms: u64,
    location: &dyn Fn(&mut Text<N>),
    message: &[u8],
) {
    text.str(&setup.header).str("at_ms: ").dec(at_ms).str("\nthread: ");
    let thread = current_thread();
    if thread == setup.main_thread {
        text.str("main");
    } else {
        text.dec(thread);
    }
    text.str("\nlocation: ");
    location(text);
    text.str("\nmessage: ").bytes(message).str("\n\n").bytes(message).str("\n\nbacktrace:\n");
}

#[cfg(unix)]
mod unix {
    use super::{first_report, now_ms, report_head, Setup, Text, SETUP};
    use libc::{c_int, c_void};

    /// The signals a crash in native code dies of.
    const SIGNALS: [c_int; 6] =
        [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT, libc::SIGTRAP];

    /// The actions in place before the handler, put back once it has written its report.
    pub(super) struct Previous([libc::sigaction; SIGNALS.len()]);

    impl Previous {
        pub(super) fn read() -> Self {
            // SAFETY: an all-zero sigaction is valid (SIG_DFL, no flags), and sigaction with a null
            // new action only reads the current one into it.
            Self(SIGNALS.map(|signal| unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                libc::sigaction(signal, std::ptr::null(), &mut action);
                action
            }))
        }

        fn of(&self, signal: c_int) -> Option<&libc::sigaction> {
            SIGNALS.iter().position(|&s| s == signal).map(|at| &self.0[at])
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn thread_id() -> u64 {
        // SAFETY: gettid takes nothing and cannot fail.
        unsafe { libc::syscall(libc::SYS_gettid) as u64 }
    }

    #[cfg(target_vendor = "apple")]
    pub(super) fn thread_id() -> u64 {
        let mut id = 0u64;
        // SAFETY: writes the current thread's id into `id`.
        unsafe { libc::pthread_threadid_np(libc::pthread_self(), &mut id) };
        id
    }

    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    pub(super) fn thread_id() -> u64 {
        // SAFETY: pthread_self takes nothing and cannot fail.
        unsafe { libc::pthread_self() as u64 }
    }

    /// The size of the stack the handler runs on in the thread that installs it, so a crash of
    /// that thread overflowing its own stack is reported too. Kept for the life of the process.
    const ALTERNATE_STACK_SIZE: usize = 256 * 1024;

    pub(super) fn install() {
        // SAFETY: sigaltstack and sigaction are given valid structures; the alternate stack is
        // leaked, so it outlives every use.
        unsafe {
            let mut current: libc::stack_t = std::mem::zeroed();
            libc::sigaltstack(std::ptr::null(), &mut current);
            // Rust gives its threads a small one, and a thread from C none.
            if current.ss_flags & libc::SS_DISABLE != 0 || current.ss_size < ALTERNATE_STACK_SIZE {
                let stack: &'static mut [u8] = Box::leak(vec![0u8; ALTERNATE_STACK_SIZE].into_boxed_slice());
                let ours = libc::stack_t {
                    ss_sp: stack.as_mut_ptr().cast(),
                    ss_size: stack.len(),
                    ss_flags: 0,
                };
                libc::sigaltstack(&ours, std::ptr::null_mut());
            }
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            {
                // The first backtrace loads the unwinder, which allocates: done here, not in the
                // handler.
                let mut frames = [std::ptr::null_mut::<c_void>(); 4];
                libc::backtrace(frames.as_mut_ptr(), frames.len() as c_int);
            }
            for signal in SIGNALS {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        }
    }

    extern "C" fn on_signal(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
        let Some(setup) = SETUP.get() else {
            return;
        };
        if first_report(signal == libc::SIGABRT) {
            // SAFETY: the kernel hands the handler a valid siginfo and context for this signal.
            unsafe { write_report(setup, signal, info, context) };
        }
        // As if the handler had not been there: the earlier action, which for a fault runs when
        // the faulting instruction runs again; the default one is raised now, and kills the
        // process when the handler returns.
        // SAFETY: sigaction and raise are async-signal-safe, and the action is one sigaction gave.
        unsafe {
            let previous = setup.unix.of(signal).copied().unwrap_or(std::mem::zeroed());
            libc::sigaction(signal, &previous, std::ptr::null_mut());
            if previous.sa_sigaction == libc::SIG_DFL {
                libc::raise(signal);
            }
        }
    }

    fn name(signal: c_int) -> &'static str {
        match signal {
            libc::SIGSEGV => "Segmentation fault (SIGSEGV)",
            libc::SIGBUS => "Bus error (SIGBUS)",
            libc::SIGILL => "Illegal instruction (SIGILL)",
            libc::SIGFPE => "Arithmetic error (SIGFPE)",
            libc::SIGABRT => "Aborted (SIGABRT)",
            libc::SIGTRAP => "Trap (SIGTRAP)",
            _ => "Signal",
        }
    }

    /// # Safety
    /// From the signal handler, with what the kernel gave it.
    unsafe fn write_report(setup: &Setup, signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
        let at_ms = now_ms();
        let mut path = Text::<1024>::new();
        path.bytes(setup.folder.as_bytes()).str("/").str(&setup.component).str("-").dec(at_ms).str("-");
        path.dec(libc::getpid() as u64).str(".txt").bytes(&[0]);
        if !path.whole() {
            return;
        }
        make_folder(setup);
        let fd = libc::open(
            path.as_bytes().as_ptr().cast(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o644 as libc::c_uint,
        );
        if fd < 0 {
            return;
        }
        let mut head = Text::<2048>::new();
        let location = |text: &mut Text<2048>| {
            if info.is_null() || signal == libc::SIGABRT {
                text.str("unknown");
            } else {
                // SAFETY: the kernel's siginfo for a fault names its address.
                let address = unsafe { (*info).si_addr() } as u64;
                text.str(if matches!(signal, libc::SIGSEGV | libc::SIGBUS) { "address " } else { "at " }).hex(address);
            }
        };
        report_head(&mut head, setup, at_ms, &location, name(signal).as_bytes());
        write_all(fd, head.as_bytes());
        // Written before the backtrace, whose walk could fault again and end the process.
        backtrace(fd, context);
        write_all(fd, b"\n");
        libc::close(fd);
    }

    /// The crash folder and the folders above it, made as `mkdir -p` would.
    unsafe fn make_folder(setup: &Setup) {
        let folder = setup.folder.as_bytes();
        let mut path = Text::<1024>::new();
        path.bytes(folder).bytes(&[0]);
        if !path.whole() {
            return;
        }
        for end in (1..=folder.len()).filter(|&end| end == folder.len() || folder[end] == b'/') {
            let saved = path.bytes[end];
            path.bytes[end] = 0;
            libc::mkdir(path.bytes.as_ptr().cast(), 0o755);
            path.bytes[end] = saved;
        }
    }

    unsafe fn write_all(fd: c_int, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
            if written <= 0 {
                return;
            }
            bytes = &bytes[written as usize..];
        }
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe fn backtrace(fd: c_int, _context: *mut c_void) {
        let mut frames = [std::ptr::null_mut::<c_void>(); 64];
        let count = libc::backtrace(frames.as_mut_ptr(), frames.len() as c_int);
        libc::backtrace_symbols_fd(frames.as_ptr(), count, fd);
    }

    /// The interrupted code's frames, from its frame pointer: Apple's ABIs keep one in every
    /// frame, and backtrace() from the alternate stack would find none of them.
    #[cfg(target_vendor = "apple")]
    unsafe fn backtrace(fd: c_int, context: *mut c_void) {
        let mut frames = [std::ptr::null_mut::<c_void>(); 64];
        let mut count = 0;
        if !context.is_null() {
            let (pc, mut fp, lr) = registers(context);
            frames[0] = pc as *mut c_void;
            count = 1;
            let start = fp;
            let mut previous = 0u64;
            while count < frames.len() && fp != 0 && fp.is_multiple_of(8) && fp > previous && fp - start < (64 << 20) {
                let next = *(fp as *const u64);
                let returned = strip(*((fp + 8) as *const u64));
                if returned == 0 {
                    break;
                }
                // A leaf function keeps its caller in the link register, not in a frame.
                if count == 1 && lr != 0 && lr != returned {
                    frames[count] = lr as *mut c_void;
                    count += 1;
                }
                frames[count] = returned as *mut c_void;
                count += 1;
                previous = fp;
                fp = next;
            }
        }
        libc::backtrace_symbols_fd(frames.as_ptr(), count as c_int, fd);
    }

    /// Return addresses without a pointer authentication code.
    #[cfg(target_vendor = "apple")]
    fn strip(address: u64) -> u64 {
        if cfg!(target_arch = "aarch64") {
            address & 0x0000_7fff_ffff_ffff
        } else {
            address
        }
    }

    /// `ucontext_t` as Darwin lays it out, as far as its machine context.
    #[cfg(target_vendor = "apple")]
    #[repr(C)]
    struct DarwinContext {
        onstack: c_int,
        sigmask: u32,
        stack: libc::stack_t,
        link: *mut c_void,
        mcsize: usize,
        mcontext: *const MachineContext,
    }

    /// `__darwin_mcontext64`, as far as its thread state.
    #[cfg(target_vendor = "apple")]
    #[repr(C)]
    struct MachineContext {
        exception: [u64; 2],
        thread: ThreadState,
    }

    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    #[repr(C)]
    struct ThreadState {
        x: [u64; 29],
        fp: u64,
        lr: u64,
        sp: u64,
        pc: u64,
    }

    #[cfg(all(target_vendor = "apple", target_arch = "x86_64"))]
    #[repr(C)]
    struct ThreadState {
        rax: u64,
        rbx: u64,
        rcx: u64,
        rdx: u64,
        rdi: u64,
        rsi: u64,
        rbp: u64,
        rsp: u64,
        r: [u64; 8],
        rip: u64,
    }

    /// The interrupted code's program counter, frame pointer and link register.
    #[cfg(target_vendor = "apple")]
    unsafe fn registers(context: *mut c_void) -> (u64, u64, u64) {
        let state = &(*(*context.cast::<DarwinContext>()).mcontext).thread;
        #[cfg(target_arch = "aarch64")]
        return (state.pc, state.fp, strip(state.lr));
        #[cfg(target_arch = "x86_64")]
        return (state.rip, state.rbp, 0);
    }

    #[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_vendor = "apple")))]
    unsafe fn backtrace(fd: c_int, _context: *mut c_void) {
        write_all(fd, b"(this platform gives none)\n");
    }
}

#[cfg(windows)]
mod windows {
    use super::{first_report, report_head, Setup, Text, SETUP};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, GENERIC_WRITE, HMODULE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, CreateFileW, WriteFile, CREATE_NEW, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        RtlCaptureStackBackTrace, SetUnhandledExceptionFilter, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};

    /// The filter in place before ours, called after it.
    static PREVIOUS_FILTER: AtomicUsize = AtomicUsize::new(0);
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    pub(super) fn thread_id() -> u64 {
        // SAFETY: takes nothing and cannot fail.
        u64::from(unsafe { GetCurrentThreadId() })
    }

    pub(super) fn now_ms() -> u64 {
        let mut now = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        // SAFETY: writes the FILETIME it is given.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let ticks = (u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime);
        // 100 ns ticks since 1601.
        ticks.saturating_sub(116_444_736_000_000_000) / 10_000
    }

    pub(super) fn install() {
        // SAFETY: the filter and the handler are functions of the right types that live as long
        // as the process.
        unsafe {
            let previous = SetUnhandledExceptionFilter(Some(on_exception));
            PREVIOUS_FILTER.store(previous.map_or(0, |filter| filter as usize), Ordering::SeqCst);
            // abort(), std::terminate and so a C++ exception nothing caught: the C runtime raises
            // SIGABRT before it ends the process.
            libc::signal(libc::SIGABRT, on_abort as extern "C" fn(libc::c_int) as libc::sighandler_t);
        }
    }

    unsafe extern "system" fn on_exception(pointers: *const EXCEPTION_POINTERS) -> i32 {
        if let Some(setup) = SETUP.get() {
            if !pointers.is_null() && !(*pointers).ExceptionRecord.is_null() && first_report(false) {
                let record = &*(*pointers).ExceptionRecord;
                let code = record.ExceptionCode as u32;
                let mut message = Text::<96>::new();
                message.str(exception_name(code)).str(" (exception ").hex(u64::from(code)).str(")");
                let location = |text: &mut Text<2048>| {
                    module_and_offset(text, record.ExceptionAddress as usize);
                    // An access violation says what it tried, and where.
                    if code == 0xC000_0005 && record.NumberParameters >= 2 {
                        let what = match record.ExceptionInformation[0] {
                            0 => " reading ",
                            8 => " executing ",
                            _ => " writing ",
                        };
                        text.str(what).hex(record.ExceptionInformation[1] as u64);
                    }
                };
                write_report(setup, &location, message.as_bytes());
            }
        }
        let previous = PREVIOUS_FILTER.load(Ordering::SeqCst);
        if previous == 0 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let previous: LPTOP_LEVEL_EXCEPTION_FILTER = Some(std::mem::transmute::<
            usize,
            unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32,
        >(previous));
        previous.map_or(EXCEPTION_CONTINUE_SEARCH, |filter| filter(pointers))
    }

    /// The C runtime has put back the default handler before calling this one, and ends the
    /// process once it returns.
    extern "C" fn on_abort(_signal: libc::c_int) {
        if let Some(setup) = SETUP.get() {
            if first_report(true) {
                let location = |text: &mut Text<2048>| {
                    text.str("unknown");
                };
                // SAFETY: from the C runtime's signal handler, in the aborting thread.
                unsafe { write_report(setup, &location, b"Aborted (SIGABRT)") };
            }
        }
    }

    fn exception_name(code: u32) -> &'static str {
        match code {
            0xC000_0005 => "Access violation",
            0xC000_00FD => "Stack overflow",
            0xC000_001D => "Illegal instruction",
            0xC000_0096 => "Privileged instruction",
            0xC000_0094 => "Integer division by zero",
            0xC000_0095 => "Integer overflow",
            0xC000_0006 => "In-page error",
            0xC000_0374 => "Heap corruption",
            0x8000_0003 => "Breakpoint",
            _ => "Exception",
        }
    }

    unsafe fn write_report(setup: &Setup, location: &dyn Fn(&mut Text<2048>), message: &[u8]) {
        let at_ms = now_ms();
        let mut path = [0u16; 1024];
        let mut length = 0;
        let mut push = |units: &mut dyn Iterator<Item = u16>| {
            for unit in units {
                if length + 1 < path.len() {
                    path[length] = unit;
                    length += 1;
                }
            }
        };
        let mut name = Text::<96>::new();
        name.str("\\").str(&setup.component).str("-").dec(at_ms).str("-").dec(u64::from(GetCurrentProcessId())).str(".txt");
        push(&mut setup.folder.iter().copied());
        push(&mut name.as_bytes().iter().map(|&byte| u16::from(byte)));
        if !name.whole() || length + 1 >= path.len() {
            return;
        }
        make_folder(setup);
        let file = CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        );
        if file == INVALID_HANDLE_VALUE {
            return;
        }
        let mut head = Text::<2048>::new();
        report_head(&mut head, setup, at_ms, location, message);
        write_all(file, head.as_bytes());
        let mut frames = [std::ptr::null_mut::<core::ffi::c_void>(); 62];
        let count = RtlCaptureStackBackTrace(0, frames.len() as u32, frames.as_mut_ptr(), std::ptr::null_mut());
        for (index, &frame) in frames[..usize::from(count)].iter().enumerate() {
            let mut line = Text::<640>::new();
            line.dec(index as u64).str(": ");
            module_and_offset(&mut line, frame as usize);
            line.str("\n");
            write_all(file, line.as_bytes());
        }
        write_all(file, b"\n");
        CloseHandle(file);
    }

    /// The crash folder and the folders above it.
    unsafe fn make_folder(setup: &Setup) {
        let folder = &setup.folder;
        let mut path = [0u16; 1024];
        if folder.len() + 1 >= path.len() {
            return;
        }
        path[..folder.len()].copy_from_slice(folder);
        let separator = |unit: u16| unit == u16::from(b'\\') || unit == u16::from(b'/');
        for end in (3..=folder.len()).filter(|&end| end == folder.len() || separator(folder[end])) {
            let saved = path[end];
            path[end] = 0;
            CreateDirectoryW(path.as_ptr(), std::ptr::null());
            path[end] = saved;
        }
    }

    /// `kerf-qt.exe+0x1a2b`, or the bare address outside any module.
    unsafe fn module_and_offset<const N: usize>(text: &mut Text<N>, address: usize) {
        let mut module: HMODULE = std::ptr::null_mut();
        let found = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        ) != 0;
        let mut name = [0u16; 520];
        let length = if found { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize } else { 0 };
        if length == 0 {
            text.hex(address as u64);
            return;
        }
        let name = &name[..length.min(name.len())];
        let base = name.iter().rposition(|&unit| unit == u16::from(b'\\') || unit == u16::from(b'/')).map_or(0, |at| at + 1);
        for decoded in char::decode_utf16(name[base..].iter().copied()) {
            let mut utf8 = [0u8; 4];
            text.str(decoded.unwrap_or(char::REPLACEMENT_CHARACTER).encode_utf8(&mut utf8));
        }
        text.str("+").hex((address - module as usize) as u64);
    }

    unsafe fn write_all(file: windows_sys::Win32::Foundation::HANDLE, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let mut written = 0u32;
            if WriteFile(file, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut()) == 0 || written == 0 {
                return;
            }
            bytes = &bytes[written as usize..];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Text;

    #[test]
    fn text_is_made_without_allocating_and_cut_where_it_does_not_fit() {
        let mut text = Text::<32>::new();
        text.str("at ").dec(0).str(" ").dec(1_762_600_000_123).str(" ").hex(0x10).str(" ").hex(0);
        assert_eq!(text.as_bytes(), b"at 0 1762600000123 0x10 0x0");
        assert!(text.whole());
        text.str("and more than fits");
        assert_eq!(text.as_bytes().len(), 32);
        assert!(!text.whole());
    }
}
