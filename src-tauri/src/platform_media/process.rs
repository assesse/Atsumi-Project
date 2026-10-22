//! A bounded, cancellable process tree. Extractor output is private, never a diagnostic log.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[cfg(windows)]
struct Tree(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn contain(child: &mut std::process::Child) -> Result<Tree, String> {
    contain_with_memory(child, None)
}

#[cfg(windows)]
fn contain_with_memory(
    child: &mut std::process::Child,
    process_memory: Option<usize>,
) -> Result<Tree, String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
    };
    unsafe {
        let handle = CreateJobObjectW(None, None).map_err(|_| "process containment unavailable")?;
        let tree = Tree(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(bytes) = process_memory {
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.ProcessMemoryLimit = bytes;
        }
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(|_| "process containment failed")?;
        AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
            .map_err(|_| "process assignment failed")?;
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
            .map_err(|_| "process thread unavailable")?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut found = false;
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == child.id() {
                    if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                    {
                        found |= ResumeThread(thread) != u32::MAX;
                        let _ = CloseHandle(thread);
                    }
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        if !found {
            return Err("process resume failed".into());
        }
        Ok(tree)
    }
}

const STREAM_INPUT_LIMIT: usize = 8 * 1024;
const STREAM_LINE_LIMIT: usize = 512 * 1024;
const STREAM_MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// The callback runs on the reader thread, with pipe backpressure and no queue.
/// The limit includes the newline. A nonempty partial final line is never success.
fn read_lines(
    mut source: impl Read,
    mut on_line: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut buffer = [0; 16 * 1024];
    let mut line = Vec::new();
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| "채팅 수신 데이터를 읽지 못했습니다.")?;
        if count == 0 {
            return if line.is_empty() {
                Ok(())
            } else {
                Err("마지막 채팅 행이 완전하지 않습니다. 받은 기록은 보존했습니다.".into())
            };
        }
        for piece in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
            if line.len().saturating_add(piece.len()) > STREAM_LINE_LIMIT {
                return Err("채팅 행이 한도를 넘었습니다.".into());
            }
            line.extend_from_slice(piece);
            if line.last() == Some(&b'\n') {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if !line.is_empty() {
                    on_line(&line)?;
                }
                line.clear();
            }
        }
    }
}

pub(super) struct StreamChild {
    child: std::process::Child,
    #[cfg(windows)]
    tree: Tree,
}
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProbeSpawnError {
    Spawn,
    Containment,
}
impl StreamChild {
    /// Private dormant packet-probe entry. Command policy stays in its fixed
    /// caller; process ownership and limits reuse the existing chat machinery.
    #[cfg(windows)]
    pub(super) fn spawn_packet_probe(mut command: Command) -> Result<Self, ProbeSpawnError> {
        use std::os::windows::process::CommandExt;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000 | 0x00000004);
        let mut child = command.spawn().map_err(|_| ProbeSpawnError::Spawn)?;
        match contain_with_memory(&mut child, Some(STREAM_MEMORY_LIMIT)) {
            Ok(tree) => Ok(Self { child, tree }),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(ProbeSpawnError::Containment)
            }
        }
    }
    #[cfg(windows)]
    pub(super) fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.child.stdin.take()
    }
    #[cfg(windows)]
    pub(super) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }
    #[cfg(windows)]
    pub(super) fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }
    #[cfg(windows)]
    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub(super) fn terminate(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::System::JobObjects::TerminateJobObject(self.tree.0, 1);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for StreamChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Chat-only streaming process. Other download/segmenter limits are unchanged.
/// `inspect` is sampled at100ms, not a byte-exact filesystem/network quota.
#[allow(clippy::too_many_arguments)]
pub(super) fn stream_lines(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    input: &[u8],
    stop: &Arc<AtomicBool>,
    timeout: Duration,
    on_line: impl FnMut(&[u8]) -> Result<(), String> + Send,
    mut inspect: impl FnMut() -> Result<(), String>,
    on_failure: impl Fn(&str) + Sync,
) -> Result<(), String> {
    let notify_failure = |error: &str| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_failure(error)));
    };
    if input.len() > STREAM_INPUT_LIMIT {
        return Err("채팅 연결 정보가 한도를 넘었습니다.".into());
    }
    if stop.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    if let Err(error) = inspect() {
        notify_failure(&error);
        return Err(error);
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "PYTHONPATH",
        "PYTHONHOME",
        "DENO_DIR",
        "NODE_OPTIONS",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(key);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000004);
    }
    let mut child = command.spawn().map_err(|_| {
        let error = "채팅 도구를 실행하지 못했습니다.";
        notify_failure(error);
        error
    })?;
    #[cfg(windows)]
    let tree = match contain_with_memory(&mut child, Some(STREAM_MEMORY_LIMIT)) {
        Ok(tree) => tree,
        Err(error) => {
            notify_failure(&error);
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let owned = StreamChild {
        child,
        #[cfg(windows)]
        tree,
    };
    let failure = std::sync::Mutex::new(None::<String>);
    let report = |result: &Result<(), String>| {
        if let Err(error) = result {
            // Latch the affected recording BEFORE waiting for any process/pipe.
            // Do not put the callback under our local failure mutex.
            notify_failure(error);
            if let Ok(mut first) = failure.lock() {
                if first.is_none() {
                    *first = Some(error.clone());
                }
            }
        }
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            // This owner drops BEFORE scope waits on borrowed reader threads,
            // including a failed thread spawn or an inspector panic.
            let mut owned = owned;
            let mut stdin = owned.child.stdin.take().ok_or_else(|| {
                let error = "채팅 입력을 연결하지 못했습니다.";
                notify_failure(error);
                error
            })?;
            let stdout = owned.child.stdout.take().ok_or_else(|| {
                let error = "채팅 출력을 연결하지 못했습니다.";
                notify_failure(error);
                error
            })?;
            let stderr = owned.child.stderr.take().ok_or_else(|| {
                let error = "채팅 상태를 연결하지 못했습니다.";
                notify_failure(error);
                error
            })?;
            let input_worker = std::thread::Builder::new()
                .spawn_scoped(scope, || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        stdin
                            .write_all(input)
                            .map_err(|_| "채팅 연결 정보를 전달하지 못했습니다.".to_string())
                    }))
                    .unwrap_or_else(|_| Err("채팅 입력 처리 중 오류가 발생했습니다.".into()));
                    // Closing stdin is necessary for --load-info-json - to reach EOF.
                    drop(stdin);
                    report(&result);
                    result
                })
                .map_err(|_| {
                    let error = "채팅 입력 처리를 시작하지 못했습니다.".to_string();
                    report(&Err(error.clone()));
                    error
                })?;
            let output_worker = std::thread::Builder::new()
                .spawn_scoped(scope, || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        read_lines(stdout, on_line)
                    }))
                    .unwrap_or_else(|_| Err("채팅 처리 중 오류가 발생했습니다.".into()));
                    report(&result);
                    result
                })
                .map_err(|_| {
                    let error = "채팅 출력 처리를 시작하지 못했습니다.".to_string();
                    report(&Err(error.clone()));
                    error
                })?;
            let error_worker = std::thread::Builder::new()
                .spawn_scoped(scope, || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        drain(stderr, 65536)
                    }))
                    .map_err(|_| "채팅 상태 처리 중 오류가 발생했습니다.".to_string());
                    if let Err(error) = &result {
                        report(&Err(error.clone()));
                    }
                    result
                })
                .map_err(|_| {
                    let error = "채팅 상태 처리를 시작하지 못했습니다.".to_string();
                    report(&Err(error.clone()));
                    error
                })?;
            let started = Instant::now();
            let status = loop {
                if let Some(error) = failure.lock().map_err(|_| "채팅 상태 오류")?.clone() {
                    break Err(error);
                }
                if stop.load(Ordering::Acquire) {
                    break Err("cancelled".into());
                }
                if started.elapsed() >= timeout {
                    let error =
                        "채팅 수신 시간이 한도에 도달했습니다. 기록은 보존했습니다.".to_string();
                    report(&Err(error.clone()));
                    break Err(error);
                }
                let inspection =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut inspect))
                        .unwrap_or_else(|_| {
                            Err("채팅 임시 조각 확인 중 오류가 발생했습니다.".into())
                        });
                if let Err(error) = inspection {
                    report(&Err(error.clone()));
                    break Err(error);
                }
                match owned.child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            report(&Err(
                                "YouTube 채팅 수신이 중단됐습니다. 받은 기록은 보존했습니다."
                                    .into(),
                            ));
                        }
                        break Ok(status);
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => {
                        let error = "채팅 도구 상태를 확인하지 못했습니다.".to_string();
                        report(&Err(error.clone()));
                        break Err(error);
                    }
                }
            };
            // Also close descendants that retained a pipe after their parent exit.
            owned.terminate();
            let output = output_worker.join().map_err(|_| "채팅 출력 처리 실패")?;
            let input = input_worker.join().map_err(|_| "채팅 입력 처리 실패")?;
            let _private_error = error_worker.join().map_err(|_| "채팅 상태 처리 실패")??;
            // Parser/callback failure outranks intentional cancellation, so a
            // truncated buffered line cannot be turned into a complete archive.
            output?;
            input?;
            if !status?.success() {
                return Err("YouTube 채팅 수신이 중단됐습니다. 받은 기록은 보존했습니다.".into());
            }
            Ok(())
        })
    }));
    outcome.unwrap_or_else(|_| Err("채팅 처리 중 오류가 발생했습니다.".into()))
}

fn drain(mut pipe: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut overflow = false;
    let mut bytes = [0; 8192];
    loop {
        match pipe.read(&mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let take = n.min(limit.saturating_sub(out.len()));
                out.extend_from_slice(&bytes[..take]);
                overflow |= take < n;
            }
        }
    }
    (out, overflow)
}

pub(super) fn run(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    cancelled: &Arc<AtomicBool>,
    timeout: Duration,
    stdout_limit: usize,
) -> Result<Output, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Do not inherit proxy credentials or Python/Deno injection settings from a shell.
    for key in [
        "PYTHONPATH",
        "PYTHONHOME",
        "DENO_DIR",
        "NODE_OPTIONS",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        command.env_remove(key);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000004);
    }
    let mut child = command.spawn().map_err(|_| "도구를 실행하지 못했습니다.")?;
    #[cfg(windows)]
    let tree = match contain(&mut child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let stdout = child.stdout.take().ok_or("도구 출력이 없습니다.")?;
    let stderr = child.stderr.take().ok_or("도구 오류 출력이 없습니다.")?;
    let a = std::thread::spawn(move || drain(stdout, stdout_limit));
    let b = std::thread::spawn(move || drain(stderr, 65536));
    let started = Instant::now();
    let mut aborted = false;
    let status = loop {
        if cancelled.load(Ordering::Acquire) || started.elapsed() > timeout {
            aborted = true;
            #[cfg(windows)]
            unsafe {
                let _ = windows::Win32::System::JobObjects::TerminateJobObject(tree.0, 1);
            }
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => break Err(error),
        }
    };
    #[cfg(windows)]
    drop(tree); // also terminates any descendant left after its parent exited
    let (stdout, overflow) = a.join().map_err(|_| "도구 출력 처리 실패")?;
    let (stderr, _) = b.join().map_err(|_| "도구 출력 처리 실패")?;
    if aborted {
        return Err(if cancelled.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "도구 응답 시간이 초과됐습니다."
        }
        .into());
    }
    if overflow {
        return Err("도구 응답이 허용 크기를 넘었습니다.".into());
    }
    Ok(Output {
        success: status
            .map_err(|_| "도구 상태를 확인하지 못했습니다.")?
            .success(),
        stdout,
        stderr,
    })
}

/// One network receiver feeds a local-only segmenter. Stopping the receiver closes
/// the pipe so the segmenter can commit its final segment before its job is closed.
pub(super) fn stream_pipeline(
    receiver: &Path,
    receiver_args: &[String],
    segmenter: &Path,
    segmenter_args: &[String],
    cwd: &Path,
    stop: &Arc<AtomicBool>,
    mut inspect: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    fn command(executable: &Path, args: &[String], cwd: &Path) -> Command {
        let mut command = Command::new(executable);
        command.args(args).current_dir(cwd).stderr(Stdio::piped());
        for key in [
            "PYTHONPATH",
            "PYTHONHOME",
            "DENO_DIR",
            "NODE_OPTIONS",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
        ] {
            command.env_remove(key);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000 | 0x00000004);
        }
        command
    }
    if stop.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let mut input = command(receiver, receiver_args, cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "라이브 수신 도구를 실행하지 못했습니다.")?;
    #[cfg(windows)]
    let input_tree = match contain(&mut input) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = input.kill();
            let _ = input.wait();
            return Err(error);
        }
    };
    let input_error = input
        .stderr
        .take()
        .ok_or("수신 상태를 확인하지 못했습니다.")?;
    let input_output = input
        .stdout
        .take()
        .ok_or("수신 데이터를 연결하지 못했습니다.")?;
    let mut output = match command(segmenter, segmenter_args, cwd)
        .stdin(Stdio::from(input_output))
        .stdout(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            let _ = input.kill();
            let _ = input.wait();
            return Err("로컬 재생 준비에 실패했습니다.".into());
        }
    };
    #[cfg(windows)]
    let output_tree = match contain(&mut output) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = input.kill();
            let _ = output.kill();
            let _ = input.wait();
            let _ = output.wait();
            return Err(error);
        }
    };
    let output_error = output
        .stderr
        .take()
        .ok_or("재생 상태를 확인하지 못했습니다.")?;
    let a = std::thread::spawn(move || drain(input_error, 65536));
    let b = std::thread::spawn(move || drain(output_error, 65536));
    let mut reason = None;
    let mut requested_stop = false;
    let mut input_done = false;
    let mut input_exit_code = None;
    let mut output_status = None;
    loop {
        if let Err(error) = inspect() {
            reason = Some(error);
            requested_stop = true;
            break;
        }
        if stop.load(Ordering::Acquire) {
            requested_stop = true;
            break;
        }
        match input.try_wait() {
            Ok(Some(status)) => {
                input_done = true;
                input_exit_code = status.code();
                if !status.success() {
                    reason = Some("라이브 수신이 중단됐습니다.".into());
                }
                break;
            }
            Ok(None) => {}
            Err(_) => {
                reason = Some("라이브 수신 상태를 확인하지 못했습니다.".into());
                break;
            }
        }
        match output.try_wait() {
            Ok(Some(status)) => {
                output_status = Some(status);
                reason = Some("로컬 재생 스트림이 중단됐습니다.".into());
                break;
            }
            Ok(None) => {}
            Err(_) => {
                reason = Some("재생 스트림 상태를 확인하지 못했습니다.".into());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    if !input_done {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::System::JobObjects::TerminateJobObject(input_tree.0, 1);
        }
        let _ = input.kill();
        input_exit_code = input.wait().ok().and_then(|status| status.code());
    }
    #[cfg(windows)]
    drop(input_tree);
    let flush = Instant::now();
    while output_status.is_none() && flush.elapsed() < Duration::from_secs(12) {
        match output.try_wait() {
            Ok(Some(status)) => output_status = Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    if output_status.is_none() {
        let _ = output.kill();
        let _ = output.wait();
        reason =
            Some("마지막 라이브 조각을 마무리하지 못했습니다. 받은 조각은 보존했습니다.".into());
    }
    #[cfg(windows)]
    drop(output_tree);
    let (input_errors, _) = a.join().unwrap_or_default();
    let _ = b.join();
    if let Err(error) = inspect() {
        reason = Some(error);
    }
    // This pinned CLI code means a strict breaking filter rejected the actual
    // receiver. Do not expose its title/filter text or call it user cancellation.
    if input_exit_code == Some(101) {
        return Err(super::broadcast::IDENTITY_ERROR.into());
    }
    if !requested_stop && !input_errors.is_empty() && reason.is_some() {
        return Err(super::classify_failure(&input_errors));
    }
    if let Some(error) = reason {
        return Err(error);
    }
    if output_status.is_some_and(|status| !status.success()) && !requested_stop {
        return Err("라이브 조각 저장이 중단됐습니다.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn stream_lines_split_crlf_and_exact_line_limit_without_partial_success() {
        let mut rows = Vec::new();
        read_lines(Cursor::new(b"first\r\n\nsecond\n"), |line| {
            rows.push(line.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(rows, vec![b"first".to_vec(), b"second".to_vec()]);
        let mut exact = vec![b'x'; STREAM_LINE_LIMIT - 1];
        exact.push(b'\n');
        let mut count = 0;
        read_lines(Cursor::new(&exact), |line| {
            count += 1;
            assert_eq!(line.len(), STREAM_LINE_LIMIT - 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
        exact.insert(0, b'x');
        assert!(read_lines(Cursor::new(&exact), |_| Ok(())).is_err());
        assert!(read_lines(Cursor::new(b"incomplete"), |_| Ok(())).is_err());
    }

    #[test]
    fn continuous_lines_exceed_old_total_budget_with_constant_reader_storage() {
        struct Repeated {
            remaining: usize,
        }
        impl Read for Repeated {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = output.len().min(self.remaining);
                output[..count].fill(b'x');
                for end in (1023..count).step_by(1024) {
                    output[end] = b'\n';
                }
                self.remaining -= count;
                Ok(count)
            }
        }
        let bytes = 128 * 1024 * 1024 + 16 * 1024;
        let mut rows = 0usize;
        read_lines(Repeated { remaining: bytes }, |line| {
            assert_eq!(line.len(), 1023);
            rows += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(rows, bytes / 1024);
    }

    #[test]
    #[ignore = "child-only fixture; invoked by exact isolated process tests"]
    fn stream_fixture_child() {
        let mut input = Vec::new();
        std::io::stdin()
            .take(STREAM_INPUT_LIMIT as u64)
            .read_to_end(&mut input)
            .unwrap();
        let mut out = std::io::stdout().lock();
        writeln!(out, "\nfixture:pid:{}", std::process::id()).unwrap();
        out.flush().unwrap();
        match input.as_slice() {
            b"split" => {
                out.write_all(b"fixture:first").unwrap();
                out.flush().unwrap();
                std::thread::sleep(Duration::from_millis(20));
                out.write_all(b"\nfixture:second\n").unwrap();
            }
            b"partial" => out.write_all(b"partial-last-line").unwrap(),
            b"long" => {
                out.write_all(&vec![b'x'; STREAM_LINE_LIMIT + 1]).unwrap();
            }
            b"stderr" => {
                std::io::stderr()
                    .write_all(&vec![b'e'; 256 * 1024])
                    .unwrap();
                out.write_all(b"fixture:done\n").unwrap();
            }
            b"fail" => {
                out.flush().unwrap();
                std::process::exit(7);
            }
            b"wait" => {
                out.write_all(b"fixture:ready\n").unwrap();
                out.flush().unwrap();
                std::thread::sleep(Duration::from_secs(10));
            }
            _ => std::process::exit(3),
        }
        out.flush().unwrap();
        std::process::exit(0);
    }

    fn child_args() -> Vec<String> {
        [
            "--exact",
            "platform_media::process::tests::stream_fixture_child",
            "--ignored",
            "--nocapture",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[cfg(windows)]
    #[test]
    fn packet_probe_owner_exposes_each_pipe_once_and_keeps_exit_status() {
        let directory = tempfile::tempdir().unwrap();
        let missing = Command::new(directory.path().join("missing-fixture.exe"));
        assert!(matches!(
            StreamChild::spawn_packet_probe(missing),
            Err(ProbeSpawnError::Spawn)
        ));
        for (mode, code) in [(b"split".as_slice(), 0), (b"fail".as_slice(), 7)] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(child_args())
                .current_dir(directory.path())
                .env_clear();
            let mut owned = StreamChild::spawn_packet_probe(command).unwrap();
            let mut input = owned.take_stdin().unwrap();
            let output = owned.take_stdout().unwrap();
            let errors = owned.take_stderr().unwrap();
            assert!(owned.take_stdin().is_none());
            assert!(owned.take_stdout().is_none());
            assert!(owned.take_stderr().is_none());
            input.write_all(mode).unwrap();
            drop(input);
            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = owned.try_wait().unwrap() {
                    break status;
                }
                assert!(Instant::now() < deadline, "fixture deadline");
                std::thread::sleep(Duration::from_millis(10));
            };
            owned.terminate();
            assert_eq!(status.code(), Some(code));
            // This is a small libtest child fixture, not packet-parser input.
            // Production packet decoding never strips a libtest preamble.
            let mut bytes = Vec::new();
            output.take(4096).read_to_end(&mut bytes).unwrap();
            assert!(bytes
                .windows(b"fixture:pid:".len())
                .any(|s| s == b"fixture:pid:"));
            let mut stderr = Vec::new();
            errors.take(4096).read_to_end(&mut stderr).unwrap();
            assert!(stderr.is_empty());
        }
    }

    #[test]
    fn real_stream_child_closes_stdin_drains_stderr_and_reports_complete_lines() {
        let directory = tempfile::tempdir().unwrap();
        for mode in [b"split".as_slice(), b"stderr".as_slice()] {
            let mut rows = Vec::new();
            stream_lines(
                &std::env::current_exe().unwrap(),
                &child_args(),
                directory.path(),
                mode,
                &Arc::new(AtomicBool::new(false)),
                Duration::from_secs(5),
                |line| {
                    if line.starts_with(b"fixture:") && !line.starts_with(b"fixture:pid:") {
                        rows.push(line.to_vec());
                    }
                    Ok(())
                },
                || Ok(()),
                |_| panic!("unexpected synthetic failure"),
            )
            .unwrap();
            if mode == b"split" {
                assert_eq!(
                    rows,
                    vec![b"fixture:first".to_vec(), b"fixture:second".to_vec()]
                );
            } else {
                assert_eq!(rows, vec![b"fixture:done".to_vec()]);
            }
        }
    }

    #[test]
    fn real_stream_child_truncation_overlong_and_nonzero_exit_latch_failure() {
        let directory = tempfile::tempdir().unwrap();
        for mode in [
            b"partial".as_slice(),
            b"long".as_slice(),
            b"fail".as_slice(),
        ] {
            let failed = AtomicBool::new(false);
            assert!(stream_lines(
                &std::env::current_exe().unwrap(),
                &child_args(),
                directory.path(),
                mode,
                &Arc::new(AtomicBool::new(false)),
                Duration::from_secs(5),
                |_| Ok(()),
                || Ok(()),
                |_| {
                    failed.store(true, Ordering::Release);
                },
            )
            .is_err());
            assert!(failed.load(Ordering::Acquire));
        }
    }

    #[cfg(windows)]
    fn running(pid: u32) -> bool {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0;
            let success = GetExitCodeProcess(handle, &mut code).is_ok();
            let _ = CloseHandle(handle);
            success && code == 259
        }
    }

    #[test]
    fn callback_failure_and_panic_latch_before_child_drain_and_do_not_leave_workers() {
        let directory = tempfile::tempdir().unwrap();
        for panic_callback in [false, true] {
            let pid = std::sync::atomic::AtomicU32::new(0);
            let failed_before_drain = AtomicBool::new(false);
            let started = Instant::now();
            let result = stream_lines(
                &std::env::current_exe().unwrap(),
                &child_args(),
                directory.path(),
                b"wait",
                &Arc::new(AtomicBool::new(false)),
                Duration::from_secs(5),
                |line| {
                    if let Some(value) = line.strip_prefix(b"fixture:pid:") {
                        pid.store(
                            std::str::from_utf8(value).unwrap().parse().unwrap(),
                            Ordering::Release,
                        );
                    }
                    if line == b"fixture:ready" {
                        assert!(!failed_before_drain.load(Ordering::Acquire));
                        assert!(!panic_callback, "synthetic callback panic");
                        return Err("synthetic callback failure".into());
                    }
                    Ok(())
                },
                || Ok(()),
                |_| {
                    #[cfg(windows)]
                    assert!(running(pid.load(Ordering::Acquire)));
                    failed_before_drain.store(true, Ordering::Release);
                },
            );
            assert!(result.is_err());
            assert!(failed_before_drain.load(Ordering::Acquire));
            assert!(started.elapsed() < Duration::from_secs(5));
            #[cfg(windows)]
            assert!(!running(pid.load(Ordering::Acquire)));
        }
    }

    #[test]
    fn real_stream_cancel_is_clean_but_watchdog_failure_is_latched_before_drain() {
        let directory = tempfile::tempdir().unwrap();
        for cancel in [false, true] {
            let stop = Arc::new(AtomicBool::new(false));
            let ready = AtomicBool::new(false);
            let failed = AtomicBool::new(false);
            let result = stream_lines(
                &std::env::current_exe().unwrap(),
                &child_args(),
                directory.path(),
                b"wait",
                &stop,
                Duration::from_secs(5),
                |line| {
                    if line == b"fixture:ready" {
                        ready.store(true, Ordering::Release);
                        if cancel {
                            stop.store(true, Ordering::Release);
                        }
                    }
                    Ok(())
                },
                || {
                    if !cancel && ready.load(Ordering::Acquire) {
                        Err("synthetic watchdog".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    failed.store(true, Ordering::Release);
                },
            );
            assert!(result.is_err());
            assert_eq!(failed.load(Ordering::Acquire), !cancel);
            if cancel {
                assert_eq!(result.unwrap_err(), "cancelled");
            }
        }
    }

    #[test]
    fn oversized_input_and_pre_cancel_do_not_start_a_child() {
        let stop = Arc::new(AtomicBool::new(false));
        assert!(stream_lines(
            Path::new("not-an-executable"),
            &[],
            Path::new("not-a-directory"),
            &vec![0; STREAM_INPUT_LIMIT + 1],
            &stop,
            Duration::from_secs(1),
            |_| Ok(()),
            || panic!("must not inspect"),
            |_| {},
        )
        .is_err());
        stop.store(true, Ordering::Release);
        assert_eq!(
            stream_lines(
                Path::new("not-an-executable"),
                &[],
                Path::new("not-a-directory"),
                b"",
                &stop,
                Duration::from_secs(1),
                |_| Ok(()),
                || panic!("must not inspect"),
                |_| {},
            )
            .unwrap_err(),
            "cancelled"
        );
    }
}
