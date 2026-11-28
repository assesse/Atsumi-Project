//! Native-only consumer of the existing source-bound relay lane. HTTP objects
//! stay in the common worker's byte credit until their complete response ends.
use super::*;
use crate::platform_media::relay_chat::{RelayChatLease, RelayChatSummary};
use crate::streaming::relay_hls::HlsDelivery;
use crate::cloud_recording::live_fingerprint::FingerprintWorker;
use crate::streaming::relay_status::FrameReferenceInspection;

/// Own every child through join, including unwind after a sidecar was attached.
/// Dropping just RelayChatLease would leave its completed handle in SharedChat.
struct Resources {
    shared: Arc<Mutex<Admission>>,
    proof: Arc<AtomicBool>,
    worker: Option<RelayRun>,
    sidecar: Option<RelayChatLease>,
    receiver: Option<OfficialReceiverLease>,
    frames: Option<FingerprintWorker>,
    settled: bool,
}
impl Resources {
    fn new(
        shared: Arc<Mutex<Admission>>,
        proof: Arc<AtomicBool>,
        receiver: Option<OfficialReceiverLease>,
    ) -> Self {
        proof.store(false, Ordering::Release);
        Self {
            shared,
            proof,
            worker: None,
            sidecar: None,
            receiver,
            frames: None,
            settled: false,
        }
    }
    fn finish_sidecar(&mut self, result: &mut Inspection, progress: &Mutex<Inspection>) {
        let Some(sidecar) = self.sidecar.as_mut() else { return; };
        sidecar.begin_finish(result.interrupted);
        loop {
            if interrupted(&self.shared) {
                result.fail("Cancelled");
                self.sidecar.as_mut().unwrap().begin_finish(true);
            }
            let outcome = self.sidecar.as_mut().unwrap().poll_join();
            match outcome {
                Ok(Some(summary)) => {
                    // The text writer is joined, but do not publish sidecarJoined
                    // until the optional source-reference owner also finishes.
                    self.finish_reference(&summary.directory, result, progress);
                    result.sidecar(summary);
                    break;
                }
                Err(_) => {
                    result.fail("SidecarJoin");
                    break;
                }
                Ok(None) => {
                    *lock(progress) = result.clone();
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        // poll_join returned only after collecting the actual writer handle.
        self.sidecar.take();
    }
    fn finish_reference(&mut self, root: &std::path::Path, result: &mut Inspection, progress: &Mutex<Inspection>) {
        let Some(frames) = self.frames.as_mut() else { return; };
        frames.begin_finish(result.interrupted);
        let root = std::fs::canonicalize(root);
        let state = if let Ok(root) = root {
            let mut publish_failures = 0;
            loop {
                if interrupted(&self.shared) { frames.begin_finish(true); }
                match frames.poll_finish_into(&root) {
                    Ok(Some(reference)) => break FrameReferenceInspection::settled(&reference),
                    Ok(None) => { *lock(progress) = result.clone(); thread::sleep(Duration::from_millis(10)); },
                    Err(_) => {
                        publish_failures += 1;
                        if publish_failures >= 3 { break FrameReferenceInspection::failed_joined(); }
                        thread::sleep(Duration::from_millis(100));
                    },
                }
            }
        } else { FrameReferenceInspection::failed_joined() };
        // Drop joins even a failed/panicking decoder before reporting joined.
        self.frames.take();
        result.frame_reference = Some(state);
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        if !self.settled {
            lock(&self.shared).stop(true);
        }
        if let Some(worker) = self.worker.as_mut() {
            worker.cancel();
            while matches!(worker.join(Duration::from_millis(10)), JoinResult::Pending) {}
        }
        self.frames.take(); // FingerprintWorker::drop cancels and joins its child/worker.
        if let Some(sidecar) = self.sidecar.as_mut() {
            sidecar.begin_finish(true);
            while matches!(sidecar.poll_join(), Ok(None)) {
                thread::sleep(Duration::from_millis(10));
            }
        }
        lock(&self.shared).input.take();
        self.proof.store(true, Ordering::Release);
    }
}
impl Inspection {
    fn sidecar(&mut self, summary: RelayChatSummary) {
        if summary.failure.is_some() || !summary.joined {
            self.fail("Sidecar");
        }
        self.interrupted |= summary.interrupted;
        self.sidecar_directory = Some(summary.directory);
        self.sidecar = Some(SidecarInspection {
            sidecar_id: summary.sidecar_id,
            joined: summary.joined,
            raw_rows: summary.raw_rows,
            event_rows: summary.event_rows,
            bytes: summary.bytes,
            interrupted: summary.interrupted,
            assets_incomplete: summary.assets_incomplete,
            failure: summary.failure.is_some(),
            text_state: summary.text_state,
            clock_quality: summary.clock_quality.to_owned(),
            timing_records: summary.timing_records,
            timing_incomplete: summary.timing_incomplete,
        });
    }
}
fn interrupted(shared: &Mutex<Admission>) -> bool {
    let mut state = lock(shared);
    if !state.stopped && state.stop_deadline.is_some_and(|d| Instant::now() >= d) {
        state.stop(true);
    }
    state.interrupted
}

pub(super) fn run_manager(
    context: Arc<Context>,
    shared: Arc<Mutex<Admission>>,
    begin: Receiver<BeginJob>,
    replies: Replies,
    cleanup_proof: Arc<AtomicBool>,
    progress: Arc<Mutex<Inspection>>,
    target: HlsTarget,
    receiver: Option<OfficialReceiverLease>,
) -> Inspection {
    let mut result = Inspection {
        run_id: context.run.to_string(),
        delivery: Some(Default::default()),
        frame_reference: Some(FrameReferenceInspection::unavailable("tools_unavailable")),
        ..Default::default()
    };
    let mut resources = Resources::new(shared.clone(), cleanup_proof, receiver);
    let deadline = Instant::now() + ARM_TIMEOUT;
    let job = loop {
        if interrupted(&shared) || Instant::now() >= deadline {
            result.fail("ArmStopped");
            *lock(&progress) = result.clone();
            replies.stop(context.run);
            return result;
        }
        match begin.recv_timeout(Duration::from_millis(10)) {
            Ok(job) => break job,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                result.fail("BeginClosed");
                *lock(&progress) = result.clone();
                replies.stop(context.run);
                return result;
            }
        }
    };
    // Only this already validated begin supplies the source UUID. The native
    // Session receiver supplies channel/broadcast/download ownership itself.
    if let Some(receiver) = resources.receiver.as_ref() {
        match receiver.attach_relay_chat(
            &context.run.to_string(),
            &job.source.to_string(),
            context.epoch,
        ) {
            Ok(sidecar) => {
                result.sidecar_owner = Some(sidecar.native_owner().clone());
                lock(&shared).timing=Some(sidecar.timing());
                resources.sidecar = Some(sidecar);
                let tools = replies.view.as_ref().and_then(|view| view.app_handle().try_state::<crate::interface::commands::AppState>()
                    .and_then(|state| state.existing_platform_media()).map(|media| media.cloud_media_tools()));
                if let Some(tools) = tools {
                    match FingerprintWorker::start(tools, context.run, job.source) {
                        Ok(frames) => {
                            resources.frames = Some(frames);
                            result.frame_reference = Some(FrameReferenceInspection::sampling());
                        },
                        Err(_) => result.frame_reference = Some(FrameReferenceInspection::unavailable("reference_worker_unavailable")),
                    }
                }
            },
            Err(_) => result.fail("SidecarAttach"),
        }
    }
    let transport_cancel = lock(&shared).transport_cancel.clone();
    let mut delivery = match HlsDelivery::new(target, transport_cancel) {
        Ok(delivery) => Some(if let Some(frames) = &resources.frames { delivery.with_frame_sampler(frames.sink()) } else { delivery }),
        Err(fault) => {
            result.delivery_failed(fault);
            None
        }
    };
    let started = if result.interrupted || interrupted(&shared) {
        None
    } else {
        Some(
            RelayBinding::new(NativeIdentity {
                provider: RelayProvider::Soop,
                owner: context.owner,
                run: context.run,
                source: job.source,
                document_epoch: context.epoch,
            })
            .and_then(|binding| {
                RelayRun::start_ts(binding.clone(), job.tracks).map(|parts| (binding, parts))
            }),
        )
    };
    let parts = match started {
        Some(Ok(parts)) => Some(parts),
        Some(Err(fault)) => {
            result.fail(&format!("{fault:?}"));
            None
        }
        None => {
            if !result.interrupted {
                result.fail("Cancelled");
            }
            None
        }
    };
    if let Some((binding, (worker, input, output))) = parts {
        resources.worker = Some(worker);
        {
            let mut state = lock(&shared);
            state.input = Some((input.clone(), binding.clone()));
            if state.stopped {
                input.cancel();
            }
        }
        let delivery = delivery
            .as_mut()
            .expect("started only with a native delivery owner");
        let mut begin_replied = false;
        let mut joined = None;
        let mut hash = Sha256::new();
        let timing=resources.sidecar.as_ref().map(|s|s.timing());
        let mut next_probe=Instant::now()+Duration::from_millis(1500);
        loop {
            if interrupted(&shared) {
                if let Some(frames) = &resources.frames { frames.begin_finish(true); }
                result.fail("Cancelled");
                resources.worker.as_mut().unwrap().cancel();
            }
            let send_begin = {
                let mut state = lock(&shared);
                if !begin_replied && input.codec_admitted() && !state.stopped {
                    state.admitted = true;
                    result.codec_admitted = true;
                    begin_replied = true;
                    true
                } else {
                    false
                }
            };
            if send_begin {
                // captureChat is the existing PAGE wire contract: the page does
                // not scrape chat. Native SharedChat has its separate sidecar.
                replies.send(&job.reply_id, Ok(json!({"runId":context.run.to_string(),"mode":"relay","nativeApproved":true,"acceptedInMemory":true,"captureChat":false})));
            }
            let now=Instant::now();
            if begin_replied&&!result.interrupted&&now>=next_probe&&!lock(&shared).stopped {
                if let Some(nonce)=timing.as_ref().and_then(|t|t.probe(now)){replies.clock_probe(context.run,nonce);}
                next_probe=now+Duration::from_millis(1500);
            }
            if joined.is_none() {
                if let JoinResult::Joined(report) =
                    resources.worker.as_mut().unwrap().join(Duration::ZERO)
                {
                    result.worker(report);
                    joined = Some(report);
                    if result.interrupted {
                        lock(&shared).stop(true);
                    }
                }
            }
            // Receive one bounded guard, then finish its response before taking
            // another. No admission/session mutex is held over network work.
            match output.try_recv() {
                Ok(guard) => {
                    if guard.binding().identity() != binding.identity() {
                        result.fail("Binding");
                    } else if let Some(segment) = guard.ts_segment() {
                        result.output_chunks += 1;
                        result.output_bytes += segment.bytes().len() as u64;
                        hash.update(segment.bytes());
                        if !result.interrupted {
                            let before=Instant::now();
                            match delivery.deliver(segment) {
                                Err(fault)=>result.delivery_failed(fault),
                                Ok(_)=>{if let Some(t)=&timing {let _=t.segment_acked(segment,before,Instant::now());}}
                            }
                            result
                                .delivery
                                .as_mut()
                                .unwrap()
                                .observe(delivery.progress());
                        }
                    } else {
                        result.fail("UnexpectedMp4Output");
                    }
                    if result.interrupted {
                        lock(&shared).stop(true);
                    }
                    // guard retains the 32MiB common byte credit until here.
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) if joined.is_some() => break,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                    thread::sleep(Duration::from_millis(1))
                }
            }
            *lock(&progress) = result.clone();
        }
        let report = joined.expect("empty mailbox is final only after actual worker join");
        if interrupted(&shared) {
            result.fail("Cancelled");
        }
        if !result.interrupted {
            match report.input_end {
                InputEnd::TsEnded(end) => {
                    let before=Instant::now();
                    match delivery.finish(&end) {
                        Err(fault)=>result.delivery_failed(fault),
                        Ok(_)=>{if let Some(t)=&timing {let _=t.end_acked(&end,before,Instant::now());}}
                    }
                    result
                        .delivery
                        .as_mut()
                        .unwrap()
                        .observe(delivery.progress());
                }
                _ => result.fail("UnfinishedTsInput"),
            }
        }
        if interrupted(&shared) {
            result.fail("Cancelled");
        }
        result.output_sha256 = format!("{:x}", hash.finalize());
        if !begin_replied {
            replies.send(&job.reply_id, Err(UNAVAILABLE.into()));
        }
        drop(output);
    } else {
        replies.send(&job.reply_id, Err(UNAVAILABLE.into()));
    }
    {
        let mut state = lock(&shared);
        if result.interrupted {
            state.stop(true);
        } else {
            state.stopped = true;
        }
        state.input.take();
        state.timing.take();
    }
    resources.finish_sidecar(&mut result, &progress);
    *lock(&progress) = result.clone();
    resources.settled = true;
    replies.stop(context.run);
    result
}

#[cfg(all(test, windows))]
#[path = "original_relay_delivery_tests.rs"]
mod tests;
