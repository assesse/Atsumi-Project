//! Private offline example process ownership. No URL or arbitrary command input.
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
};

pub const MEDIA_LIMIT: usize = 16 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const QUEUE: usize = 4;
type Check<T> = Result<T, String>;

struct Owned {
    child: Child,
    job: HANDLE,
}
impl Owned {
    fn start(exe: &Path, cwd: &Path, args: &[&str]) -> Check<Self> {
        use std::os::windows::{io::AsRawHandle, process::CommandExt};
        let mut command = Command::new(exe);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000 | 0x00000004);
        for key in [
            "FFREPORT",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(key);
        }
        command.env("TEMP", cwd).env("TMP", cwd);
        let mut child = command.spawn().map_err(|_| "spawn_failed")?;
        let job = match unsafe { CreateJobObjectW(None, None) } {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("job_create_failed".into());
            }
        };
        let owned = Self { child, job };
        unsafe {
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.ProcessMemoryLimit = MEMORY_LIMIT;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|_| "job_limit_failed")?;
            AssignProcessToJobObject(job, HANDLE(owned.child.as_raw_handle()))
                .map_err(|_| "job_assignment_failed")?;
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
                .map_err(|_| "thread_snapshot_failed")?;
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut resumed = false;
            if Thread32First(snapshot, &mut entry).is_ok() {
                loop {
                    if entry.th32OwnerProcessID == owned.child.id() {
                        if let Ok(handle) =
                            OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                        {
                            resumed |= ResumeThread(handle) != u32::MAX;
                            let _ = CloseHandle(handle);
                        }
                    }
                    if Thread32Next(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
            if !resumed {
                return Err("child_resume_failed".into());
            }
        }
        Ok(owned)
    }
    fn stop(&mut self) {
        unsafe {
            let _ = TerminateJobObject(self.job, 1);
        }
        let _ = self.child.kill();
    }
    fn peak_memory(&self) -> Check<usize> {
        unsafe {
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            QueryInformationJobObject(
                Some(self.job),
                JobObjectExtendedLimitInformation,
                &mut limits as *mut _ as _,
                std::mem::size_of_val(&limits) as u32,
                None,
            )
            .map_err(|_| "job_memory_read_failed")?;
            Ok(limits.PeakProcessMemoryUsed)
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.stop();
        }
        let _ = self.child.wait();
        unsafe {
            let _ = CloseHandle(self.job);
        }
    }
}

pub struct Output {
    pub bytes: Vec<u8>,
    pub facts: Value,
}

pub fn run(
    exe: &Path,
    cwd: &Path,
    args: &[&str],
    input: Option<Arc<Vec<u8>>>,
    limit: usize,
    slow: bool,
) -> Check<Output> {
    if limit > MEDIA_LIMIT || input.as_ref().is_some_and(|v| v.len() > MEDIA_LIMIT) {
        return Err("media_bound".into());
    }
    let mut owned = Owned::start(exe, cwd, args)?;
    let stdin = owned.child.stdin.take().ok_or("stdin_missing")?;
    let mut stdout = owned.child.stdout.take().ok_or("stdout_missing")?;
    let mut stderr = owned.child.stderr.take().ok_or("stderr_missing")?;
    let stop = AtomicBool::new(false);
    let stalled = AtomicBool::new(false);
    let full = AtomicU64::new(0);
    let written = AtomicU64::new(0);
    let start = Instant::now();
    thread::scope(|scope| -> Check<Output> {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
        let stop_ref = &stop;
        let full_ref = &full;
        let written_ref = &written;
        let stalled_ref = &stalled;
        let feeder = scope.spawn(move || {
            if let Some(input) = input {
                for source in input.chunks(CHUNK) {
                    let mut chunk = source.to_vec();
                    loop {
                        if stop_ref.load(Ordering::Acquire) {
                            return;
                        }
                        match tx.try_send(chunk) {
                            Ok(()) => break,
                            Err(mpsc::TrySendError::Disconnected(_)) => return,
                            Err(mpsc::TrySendError::Full(value)) => {
                                chunk = value;
                                full_ref.fetch_add(1, Ordering::Relaxed);
                                thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                }
            }
        });
        let writer = scope.spawn(move || -> Check<()> {
            let mut stdin = stdin;
            while !stop_ref.load(Ordering::Acquire) {
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(chunk) => {
                        stdin.write_all(&chunk).map_err(|_| "input_pipe_closed")?;
                        written_ref.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        });
        let reader = scope.spawn(move || -> Check<Vec<u8>> {
            let mut bytes = Vec::new();
            let mut block = [0u8; CHUNK];
            loop {
                let count = stdout.read(&mut block).map_err(|_| "output_pipe_read")?;
                if count == 0 {
                    return Ok(bytes);
                }
                if bytes.len().saturating_add(count) > limit {
                    stop_ref.store(true, Ordering::Release);
                    return Err("output_bound_exceeded".into());
                }
                bytes.extend_from_slice(&block[..count]);
                if slow {
                    stalled_ref.store(true, Ordering::Release);
                    while !stop_ref.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    return Ok(bytes);
                }
            }
        });
        let errors = scope.spawn(move || -> Check<Vec<u8>> {
            let mut kept = Vec::new();
            let mut block = [0; 4096];
            loop {
                let n = stderr.read(&mut block).map_err(|_| "stderr_read")?;
                if n == 0 {
                    return Ok(kept);
                }
                let keep = (DIAGNOSTIC_LIMIT - kept.len()).min(n);
                kept.extend_from_slice(&block[..keep]);
            }
        });
        let mut stall_at = None;
        let mut cancelled_at = None;
        let mut timeout = false;
        let status = loop {
            match owned.child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(_) => break Err("child_wait"),
            }
            if stalled.load(Ordering::Acquire) && stall_at.is_none() {
                stall_at = Some(Instant::now());
            }
            let slow_due = stall_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(1500));
            if cancelled_at.is_none()
                && (stop.load(Ordering::Acquire)
                    || slow_due
                    || start.elapsed() > Duration::from_secs(45))
            {
                timeout = start.elapsed() > Duration::from_secs(45);
                stop.store(true, Ordering::Release);
                cancelled_at = Some(Instant::now());
                owned.stop();
            }
            if cancelled_at.is_some_and(|at| at.elapsed() > Duration::from_secs(8)) {
                break Err("owned_child_cancel_deadline");
            }
            thread::sleep(Duration::from_millis(10));
        };
        stop.store(true, Ordering::Release);
        if status.is_err() {
            owned.stop();
            let _ = owned.child.wait();
        }
        // Terminating our contained child closes its pipes before joining workers.
        feeder.join().map_err(|_| "feeder_panicked")?;
        let write_result = writer.join().map_err(|_| "writer_panicked")?;
        let bytes = reader.join().map_err(|_| "reader_panicked")??;
        let errors = errors.join().map_err(|_| "stderr_panicked")??;
        let status = status?;
        let peak = owned.peak_memory()?;
        if timeout {
            return Err("child_timeout".into());
        }
        if !slow {
            write_result?;
            if !status.success() {
                return Err(format!(
                    "ffmpeg_failed:{}",
                    String::from_utf8_lossy(&errors)
                        .chars()
                        .take(4096)
                        .collect::<String>()
                ));
            }
        } else if cancelled_at.is_none()
            || full.load(Ordering::Relaxed) == 0
            || !stalled.load(Ordering::Relaxed)
        {
            return Err("backpressure_not_observed".into());
        }
        if peak == 0 || peak > MEMORY_LIMIT {
            return Err("child_memory_bound".into());
        }
        Ok(Output {
            bytes,
            facts: json!({
                "success":status.success(),"exitCode":status.code(),"elapsedMs":start.elapsed().as_millis(),
                "peakProcessMemoryBytes":peak,"processMemoryLimitBytes":MEMORY_LIMIT,
                "inputBytesWritten":written.load(Ordering::Relaxed),"queueCapacityBytes":QUEUE*CHUNK,
                "queueFullObservations":full.load(Ordering::Relaxed),"slowConsumer":slow,
                "cancelled":cancelled_at.is_some(),"cancelJoinMs":cancelled_at.map(|at|at.elapsed().as_millis()),
                "stderrRetainedBytes":errors.len(),"outputLimitBytes":limit
            }),
        })
    })
}
