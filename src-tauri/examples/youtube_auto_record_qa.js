// Runs only from the isolated example's exact configured main origin.
// Uses production IPC, never mock capture, credentials, account or remote-page JS.
(async () => {
  const qaStarted = performance.now();
  const result = { kind: 'youtube_auto_record_qa', success: false, isolated: true, stage: 'startup' };
  const invoke = (name, args) => window.__TAURI_INTERNALS__.invoke(name, args);
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const until = async (read, accept, milliseconds) => {
    const deadline = Date.now() + milliseconds; let value, lastError;
    while (Date.now() < deadline) {
      try { value = await read(); if (accept(value)) return value; } catch (error) { lastError = error; }
      await sleep(500);
    }
    throw new Error('Timed out at ' + result.stage + (lastError ? ': ' + String(lastError).slice(0, 180) : ''));
  };
  const playbackTokens = new Set();
  const uuid = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
  const bounded = (promise, deadline, failure) => new Promise((resolve, reject) => {
    const remaining = deadline - performance.now();
    if (remaining <= 0) { Promise.resolve(promise).catch(() => {}); reject(failure); return; }
    const timer = setTimeout(() => reject(failure), remaining);
    Promise.resolve(promise).then(value => {
      clearTimeout(timer);
      if (performance.now() >= deadline) reject(failure); else resolve(value);
    }, () => { clearTimeout(timer); reject(failure); });
  });
  const closePlayback = async playbackToken => {
    await bounded(invoke('platform_media_close', { token: playbackToken }).then(() => {
      playbackTokens.delete(playbackToken);
    }), performance.now() + 2000, 'close_failed');
  };
  const verifyPlayback = async (job, loopDeadline) => {
    const deadline = Math.min(loopDeadline, performance.now() + 30000);
    const row = { jobId: uuid(job?.jobId) ? job.jobId : null, stage: 'opening', outcome: 'invalid_facts',
      duration: null, width: 0, height: 0, frames: null, errorCode: null, closed: false,
      events: { metadata: 0, play: 0, timeupdate: 0, seeked: 0, ended: 0 },
      initial: null, middle: null, tail: null, replay: null, middleSeekError: null, replaySeekError: null, eofError: null };
    let video, lease, expectedUrl, abandoned = false;
    const listeners = [];
    const failures = new Set(['budget_exhausted', 'open_failed', 'identity_mismatch', 'metadata_timeout',
      'playback_timeout', 'seek_timeout', 'eof_timeout', 'replay_timeout', 'media_error', 'play_rejected',
      'invalid_facts', 'unsupported_short_clip', 'frame_counter_unavailable', 'close_failed']);
    const check = () => {
      if (performance.now() >= deadline) throw 'budget_exhausted';
      if (video?.error) {
        const code = video.error.code;
        row.errorCode = Number.isInteger(code) && code >= 1 && code <= 4 ? code : null;
        throw 'media_error';
      }
      if (video && expectedUrl && (video.src !== expectedUrl || (video.currentSrc && video.currentSrc !== expectedUrl))) throw 'identity_mismatch';
    };
    const frameCount = () => {
      let count, kind;
      if (Number.isSafeInteger(video.webkitDecodedFrameCount) && video.webkitDecodedFrameCount >= 0) {
        count = video.webkitDecodedFrameCount; kind = 'decoded';
      } else {
        const q = video.getVideoPlaybackQuality?.();
        if (!q || !Number.isSafeInteger(q.totalVideoFrames) || !Number.isSafeInteger(q.droppedVideoFrames)
          || q.droppedVideoFrames < 0 || q.totalVideoFrames < q.droppedVideoFrames) throw 'frame_counter_unavailable';
        count = q.totalVideoFrames - q.droppedVideoFrames; kind = 'nondropped';
      }
      if (row.frames !== null && row.frames !== kind) throw 'invalid_facts';
      row.frames = kind;
      return count;
    };
    const facts = () => {
      check();
      const time = video.currentTime, duration = video.duration;
      if (!Number.isFinite(time) || time < 0 || time > 86400 || !Number.isFinite(duration) || duration <= 0 || duration > 86400
        || !Number.isInteger(video.videoWidth) || !Number.isInteger(video.videoHeight)
        || video.videoWidth <= 0 || video.videoHeight <= 0 || video.videoWidth > 16384 || video.videoHeight > 16384) throw 'invalid_facts';
      row.duration = duration; row.width = video.videoWidth; row.height = video.videoHeight;
      return { time, frames: frameCount() };
    };
    const wait = (accept, milliseconds, failure) => new Promise((resolve, reject) => {
      const end = Math.min(deadline, performance.now() + milliseconds);
      let timer, interval;
      const finish = (error, value) => { clearTimeout(timer); clearInterval(interval); error ? reject(error) : resolve(value); };
      const tick = () => {
        try {
          check();
          if (performance.now() >= end) throw failure;
          const value = accept();
          if (value) finish(null, value);
        } catch (error) { finish(error); }
      };
      timer = setTimeout(() => finish(failure), Math.max(0, end - performance.now()));
      interval = setInterval(tick, 50); tick();
    });
    const seek = async (target, failure = 'seek_timeout') => {
      check(); video.pause();
      const before = row.events.seeked;
      video.currentTime = target;
      await wait(() => row.events.seeked > before && !video.seeking && Math.abs(video.currentTime - target) <= 0.250, 5000, failure);
      check(); return Math.abs(video.currentTime - target);
    };
    const advance = async (toEnd, failure) => {
      const first = facts(), events = { ...row.events };
      let previous = first;
      await bounded(video.play(), deadline, 'play_rejected'); check();
      return wait(() => {
        const current = facts();
        if (current.time < previous.time || current.frames < previous.frames) throw 'invalid_facts';
        previous = current;
        const delta = { seconds: current.time - first.time, frames: current.frames - first.frames };
        if (row.events.play <= events.play || row.events.timeupdate <= events.timeupdate || video.seeking || video.readyState < 2) return false;
        if (toEnd) {
          if (row.events.ended <= events.ended || !video.ended || delta.seconds <= 0 || delta.frames <= 0) return false;
          row.eofError = Math.abs(video.currentTime - video.duration);
          return row.eofError <= 0.100 ? delta : false;
        }
        return !video.paused && !video.ended && delta.seconds >= 1 && delta.frames > 0 ? delta : false;
      }, toEnd ? 8000 : 6000, failure);
    };
    try {
      check();
      if (!row.jobId || job.state !== 'completed' || job.library?.libraryId !== job.jobId
        || job.reference?.provider !== 'youtube' || job.library.reference?.provider !== 'youtube') throw 'identity_mismatch';
      const opening = invoke('platform_media_open', { libraryId: job.library.libraryId }).then(value => {
        if (uuid(value?.token)) {
          playbackTokens.add(value.token);
          if (abandoned) void closePlayback(value.token).catch(() => {});
        }
        return value;
      });
      lease = await bounded(opening, deadline, 'open_failed'); check();
      result.reopened++;
      if (!uuid(lease?.token) || lease.item?.libraryId !== job.library.libraryId
        || lease.item.reference?.provider !== 'youtube' || lease.item.reference.kind !== job.reference.kind
        || lease.item.reference.externalId !== job.reference.externalId
        || job.library.reference.kind !== job.reference.kind || job.library.reference.externalId !== job.reference.externalId
        || lease.url !== `http://platform-media.localhost/${lease.token}/media.mp4`) throw 'identity_mismatch';
      expectedUrl = lease.url;
      video = document.createElement('video');
      video.muted = true; video.defaultMuted = true; video.playsInline = true;
      video.preload = 'auto'; video.loop = false; video.playbackRate = 1;
      video.width = 640; video.height = 360;
      for (const [event, key] of [['loadedmetadata', 'metadata'], ['play', 'play'], ['timeupdate', 'timeupdate'], ['seeked', 'seeked'], ['ended', 'ended']]) {
        const listener = event => { if (event.isTrusted) row.events[key] = Math.min(100000, row.events[key] + 1); };
        video.addEventListener(event, listener); listeners.push([event, listener]);
      }
      document.body.appendChild(video);
      row.stage = 'metadata'; video.src = expectedUrl; video.load();
      await wait(() => {
        if (!row.events.metadata) return false;
        if (!Number.isFinite(video.duration) || video.duration <= 0) throw 'invalid_facts';
        if (video.duration < 4) throw 'unsupported_short_clip';
        if (video.readyState < 2 || !video.videoWidth || !video.videoHeight) return false;
        facts(); return true;
      }, 10000, 'metadata_timeout');
      row.stage = 'initial'; row.initial = await advance(false, 'playback_timeout');
      row.stage = 'middle'; row.middleSeekError = await seek(video.duration / 2);
      row.middle = await advance(false, 'playback_timeout');
      row.stage = 'eof'; await seek(video.duration - Math.min(1.5, video.duration / 4));
      row.tail = await advance(true, 'eof_timeout');
      row.stage = 'replay'; row.replaySeekError = await seek(0, 'replay_timeout');
      row.replay = await advance(false, 'replay_timeout');
      row.stage = 'complete'; row.outcome = 'complete';
    } catch (error) { row.outcome = failures.has(error) ? error : 'invalid_facts'; }
    finally {
      abandoned = true;
      if (video) {
        try {
          video.pause();
          for (const [event, listener] of listeners) video.removeEventListener(event, listener);
          video.removeAttribute('src'); video.load();
        } catch { row.outcome = 'close_failed'; }
        finally { video.remove(); }
      }
      if (uuid(lease?.token)) {
        try { await closePlayback(lease.token); row.closed = true; }
        catch { row.outcome = 'close_failed'; }
      }
    }
    return row;
  };
  let identity, token;
  const stopReservation = async () => {
    if (!identity) return;
    for (let attempt = 0; attempt < 4; attempt++) {
      const current = await invoke('youtube_auto_record_snapshot');
      if (!current.channels.some(item => item.channelId === identity.channelId && item.enabled)) return;
      try { await invoke('youtube_auto_record_set', { ...identity, enabled: false, expectedRevision: current.revision }); return; }
      catch (error) { if (attempt === 3) throw error; }
    }
  };
  try {
    const initial = await until(() => invoke('youtube_auto_record_snapshot'), value => value.ready, 30000);
    if (initial.channels.length || initial.error) throw new Error('Isolation was not empty and ready');
    result.defaultOff = true;
    result.stage = 'identity';
    identity = await invoke('youtube_auto_record_resolve', { input: __CHANNEL__ });
    result.stage = 'explicit_opt_in';
    await invoke('youtube_auto_record_set', { ...identity, enabled: true, expectedRevision: initial.revision });
    result.stage = 'automatic_record_ack';
    await until(() => invoke('youtube_auto_record_snapshot'), value => {
      const row = value.channels.find(item => item.channelId === identity.channelId);
      if (row?.status === 'failed' || value.error) throw new Error(row?.error || value.error);
      return !!row && row.recordingCount > 0;
    }, 75000);
    const jobs = await invoke('platform_media_jobs', { provider: 'youtube' });
    const recording = jobs.find(job => job.state === 'recording');
    if (!recording) throw new Error('Scheduler status lacked actual recording job');
    result.recordingCount = jobs.filter(job => job.state === 'recording').length;
    result.stage = 'attach_same_receiver';
    const watch = await invoke('youtube_original_open', { channelId: recording.reference.externalId, requestId: crypto.randomUUID(), playbackKind: 'live' });
    token = watch.token;
    await invoke('youtube_original_viewport', { token, viewport: { sequence: 1, player: { x: 0, y: 0, width: 960, height: 720 }, chat: { x: 970, y: 0, width: 320, height: 720 }, visible: true, muted: true, occlusions: [] } });
    const playing = await until(() => invoke('youtube_original_snapshot', { token }), value => value.recordingJob === recording.jobId && value.chatVisible && value.state === 'playing', 15000);
    result.sameReceiverAttached = playing.recordingJob === recording.jobId;
    result.originalChatVisible = playing.chatVisible;
    const observedJobs = new Set([recording.jobId]);
    const captureDeadline = Date.now() + 40000;
    let beforeStop = playing;
    while (Date.now() < captureDeadline) {
      await sleep(500);
      beforeStop = await invoke('youtube_original_snapshot', { token });
      if (beforeStop.recordingJob) observedJobs.add(beforeStop.recordingJob);
    }
    result.formatRolloverObserved = observedJobs.size > 1;
    result.recordingBeforeStop = !!beforeStop.recordingJob;
    result.stage = 'explicit_off'; await stopReservation();
    result.stage = 'verified_archive';
    const finished = await until(() => invoke('platform_media_jobs', { provider: 'youtube' }), values => values.length > 0 && values.every(value => ['completed', 'failed', 'interrupted'].includes(value.state)), 75000);
    result.completed = finished.filter(job => job.state === 'completed').length;
    result.failed = finished.filter(job => job.state !== 'completed').length;
    result.partialCaptures = finished.filter(job => job.library?.captureOutcome && job.library.captureOutcome.state !== 'complete').length;
    result.videoBytes = finished.reduce((total, job) => total + (job.library?.bytes || 0), 0);
    result.chatComplete = finished.every(job => job.library?.chatArchive?.state === 'complete');
    result.reopened = 0;
    const after = await invoke('youtube_auto_record_snapshot');
    result.disabled = after.channels.every(item => !item.enabled);
    result.watchSurvivesStop = (await invoke('youtube_original_snapshot', { token })).state === 'playing';
    await sleep(2000);
    const settledAgain = await invoke('platform_media_jobs', { provider: 'youtube' });
    const afterStop = await invoke('youtube_original_snapshot', { token });
    result.stoppedRemainsStopped = !afterStop.recordingJob && settledAgain.length === finished.length &&
      settledAgain.every(job => finished.some(previous => previous.jobId === job.jobId) && ['completed', 'failed', 'interrupted'].includes(job.state));
    result.archiveFlowVerified = result.completed > 0 && result.failed === 0 && result.partialCaptures === 0 && result.chatComplete && result.sameReceiverAttached && result.disabled && result.watchSurvivesStop && result.recordingBeforeStop && result.stoppedRemainsStopped;
    result.stage = 'close_original_before_replay';
    await invoke('youtube_original_close', { token }); token = null;
    result.stage = 'local_playback'; result.playback = [];
    const playbackDeadline = Math.min(performance.now() + 60000, qaStarted + 250000);
    for (const job of finished.filter(job => job.state === 'completed').slice(0, 8)) {
      if (performance.now() >= playbackDeadline) break;
      const row = await verifyPlayback(job, playbackDeadline);
      result.playback.push(row);
      if (!row.closed) break; // Never acquire another lease while one may remain.
    }
    result.playbackTested = result.playback.length;
    result.playbackSkipped = result.completed - result.playbackTested;
    const covered = result.completed > 0 && result.playbackTested === result.completed;
    result.nativePlaybackVerified = covered && result.playback.every(row => row.closed && row.initial !== null);
    result.nativeSeekVerified = covered && result.playback.every(row => row.closed && row.middle !== null);
    result.nativeEofReplayVerified = covered && result.playback.every(row => row.closed && row.outcome === 'complete');
    result.success = result.archiveFlowVerified && result.nativePlaybackVerified && result.nativeSeekVerified && result.nativeEofReplayVerified;
    result.stage = result.success ? 'complete' : result.archiveFlowVerified ? 'playback_verification' : 'archive_verification';
  } catch (error) { result.error = String(error).slice(0, 400); }
  finally {
    try { await stopReservation(); } catch { result.cleanupError = 'reservation off failed'; result.success = false; }
    if (token) try { await invoke('youtube_original_close', { token }); } catch { result.cleanupError = 'watch close failed'; result.success = false; }
    for (const playbackToken of [...playbackTokens]) {
      try { await closePlayback(playbackToken); }
      catch { result.cleanupError = 'playback close failed'; result.success = false; }
    }
    try {
      let report = JSON.stringify(result);
      if (new TextEncoder().encode(report).byteLength > 8192) {
        result.success = false; result.nativePlaybackVerified = false; result.nativeSeekVerified = false; result.nativeEofReplayVerified = false;
        result.playback = []; result.playbackTested = 0; result.playbackSkipped = result.completed ?? 0;
        result.error = 'playback report exceeded bound'; report = JSON.stringify(result);
      }
      const checked = await bounded(fetch('__REPORT__', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: report }).then(response => response.json()), Math.min(performance.now() + 45000, qaStarted + 290000), 'report_timeout');
      result.recordingFlowVerified = checked.recordingFlowVerified === true;
      result.fullContinuityVerified = checked.fullContinuityVerified === true;
      result.nativePlaybackVerified = checked.nativePlaybackVerified === true;
      result.nativeSeekVerified = checked.nativeSeekVerified === true;
      result.nativeEofReplayVerified = checked.nativeEofReplayVerified === true;
      if (checked.success !== true) result.success = false;
    } catch { result.success = false; result.error = 'bounded report failed'; }
    // Normal application shutdown drains only this new isolated identity.
    for (let attempt = 0; attempt < 5; attempt++) {
      try {
        const response = await invoke('app_active_work_snapshot');
        if (!response.ok) throw new Error('active-work snapshot unavailable');
        const quit = await invoke('app_quit', { request: { expectedWorkSetFingerprint: response.data.workSetFingerprint, confirmActiveWork: true, forceWhenStatusUnknown: false } });
        if (quit.ok && quit.data.accepted) return;
      } catch { /* Retry only graceful shutdown, never process termination. */ }
      await sleep(1000);
    }
    await bounded(fetch('__REPORT__', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...result, success: false, cleanupError: 'normal quit was not acknowledged; manually close isolated QA app' }) }), performance.now() + 2000, 'report_timeout').catch(() => {});
  }
})();
