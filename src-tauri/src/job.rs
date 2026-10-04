//! Ties karlsend's lifetime to Karlsen Desktop's.
//!
//! On Windows a child process outlives its parent when the parent is killed
//! (Task Manager, Ctrl+C on `cargo tauri dev`, a crash). An orphaned karlsend
//! keeps the RocksDB LOCK and every later start fails. Each karlsend is put in a
//! Job Object with KILL_ON_JOB_CLOSE: the job handle lives as long as Desktop,
//! and Windows terminates the node the moment Desktop's process ends, however
//! it ends. On other platforms this is a no-op.

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    /// Job handle owned for the whole process lifetime; never closed explicitly.
    struct Job(HANDLE);
    // SAFETY: a job object handle may be used from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Result<Job, u32>> = OnceLock::new();

    fn job() -> Result<HANDLE, String> {
        let job = JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return Err(GetLastError());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let e = GetLastError();
                CloseHandle(h);
                return Err(e);
            }
            Ok(Job(h))
        });
        match job {
            Ok(j) => Ok(j.0),
            Err(code) => Err(format!("cannot create job object (Windows error {code})")),
        }
    }

    pub fn bind_to_desktop_lifetime(pid: u32) -> Result<(), String> {
        let job = job()?;
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                return Err(format!("cannot open karlsend process {pid} (Windows error {})", GetLastError()));
            }
            let ok = AssignProcessToJobObject(job, process);
            let err = GetLastError();
            CloseHandle(process);
            if ok == 0 {
                return Err(format!("cannot assign karlsend to job object (Windows error {err})"));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use imp::bind_to_desktop_lifetime;

/// Non-Windows: nothing to do (stop() and the shutdown hook handle the child).
#[cfg(not(windows))]
pub fn bind_to_desktop_lifetime(_pid: u32) -> Result<(), String> {
    Ok(())
}
