import { useEffect, useRef, useState } from 'react';
import { mediaJobActive, mediaStateLabel, type MediaJob } from './platformMedia';
import { capturePresentation } from './mediaCaptureOutcome';
import '../../components/WorkQueuePanel.css';

type Section = 'queued' | 'active' | 'settled';
const sectionOf = (job: MediaJob): Section => job.state === 'queued' ? 'queued' : mediaJobActive(job) ? 'active' : 'settled';

/** Reuse presentation only; media jobs never become Hitomi queue records. */
export function PlatformMediaJobs({ jobs, privacyMode, stale, busy, onCancel, onRetry }: {
  jobs: MediaJob[]; privacyMode: boolean; stale: boolean; busy: boolean;
  onCancel(job: MediaJob): void; onRetry(job: MediaJob): void;
}) {
  const [section, setSection] = useState<Section>('active');
  const waiting = jobs.filter(job => sectionOf(job) === 'queued').length;
  const working = jobs.filter(job => sectionOf(job) === 'active').length;
  const partial = jobs.filter(job => job.state === 'completed' && capturePresentation(job.library)?.normal === false).length;
  const completed = jobs.filter(job => ['completed', 'cancelled', 'deleted'].includes(job.state)).length - partial;
  const failed = jobs.length - waiting - working - completed;
  const settled = completed + failed;
  const percent = jobs.length ? Math.floor(settled / jobs.length * 100) : 0;
  const segments = [['complete', '완료', completed], ['active', '작업 중', working], ['waiting', '대기', waiting], ['error', partial ? '일부·실패·중단' : '실패·중단', failed]] as const;
  const edge = useRef<HTMLSpanElement>(null), count = useRef<HTMLElement>(null);
  const scope = jobs.map(job => job.jobId).sort().join(',');
  const previous = useRef({ completed, scope, stale });
  useEffect(() => {
    const before = previous.current; previous.current = { completed, scope, stale };
    // The existing progress design highlights real settlement, not initial load or re-scope.
    if (stale || before.stale || before.scope !== scope || completed <= before.completed || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
    const animations = [
      edge.current?.animate?.([{ opacity: 0, transform: 'translateX(-14px)' }, { opacity: .85, offset: .3 }, { opacity: 0, transform: 'translateX(0)' }], { duration: 850, easing: 'ease-out' }),
      count.current?.animate?.([{ opacity: .5, transform: 'translateY(3px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 380, easing: 'ease-out' }),
    ];
    return () => animations.forEach(animation => animation?.cancel());
  }, [completed, scope, stale]);
  if (!jobs.length) return null;
  return <section className="platform-media-work" aria-label="영상 작업">
    <section className={`queue-summary${working && !stale ? ' is-running' : ''}${stale ? ' is-stale' : ''}${completed === jobs.length ? ' is-finished' : ''}`} aria-label="다운로드 진행 요약">
      <div className="queue-summary-heading"><span>최근 작업</span><span className="queue-summary-count" title="진행 중인 작업과 최근 처리 기록 · 취소·삭제도 완료에 포함"><strong ref={count}>{completed}</strong> / {jobs.length} 완료</span></div>
      <div className="queue-progress-track" role="group" aria-label={`자동 처리 종료 ${percent}% (취소·실패·중단 포함)`}>
        {segments.map(([tone, label, count]) => <span key={tone} className={`queue-progress-segment tone-${tone}`} style={{ width: `${count / jobs.length * 100}%` }} role="img" aria-hidden={!count} aria-label={`${label} ${count}개`} title={`${label} ${count}개`}>{tone === 'complete' ? <span ref={edge} className="queue-completion-edge" aria-hidden="true" /> : null}</span>)}
      </div>
      <div className="queue-summary-legend">{segments.map(([tone, label, count]) => <span key={tone} className={`tone-${tone}`}><i className="queue-legend-dot" aria-hidden="true" />{label} <b>{count}</b></span>)}</div>
      {stale ? <p className="queue-error">갱신 지연 · 마지막 확인값입니다.</p> : null}
    </section>
    <div className="platform-media-work-tabs" role="tablist" aria-label="작업 상태">
      {([['queued', '대기 큐', waiting], ['active', '작업 중', working], ['settled', '완료', settled]] as const).map(([key, label, count]) => <button key={key} type="button" role="tab" aria-selected={section === key} onClick={() => setSection(key)}>{label} <span>{count}</span></button>)}
    </div>
    <div className="platform-media-jobs" role="tabpanel" aria-label={section === 'queued' ? '대기 큐' : section === 'active' ? '작업 중' : '완료'}>
      {jobs.filter(job => sectionOf(job) === section).map(job => <div key={job.jobId}>
        <div><strong>{privacyMode ? '프라이버시 모드' : job.title || job.reference.externalId}</strong>
          {job.state === 'downloading' && job.receivedBytes > 0 ? <small>{(job.receivedBytes / 1e6).toFixed(1)} MB{job.totalBytes && job.totalBytes > 0 ? ` / ${(job.totalBytes / 1e6).toFixed(1)} MB` : ''} 다운로드</small> : null}
          {job.error && !privacyMode ? <small className="platform-media-error">{job.error}</small> : null}
        </div>
        <div className="platform-media-job-state">{mediaJobActive(job) ? <button type="button" disabled={busy || stale} onClick={() => onCancel(job)}>취소</button> : ['failed', 'interrupted', 'cancelled'].includes(job.state) ? <button type="button" disabled={busy || stale} onClick={() => onRetry(job)}>{job.reference.kind === 'live' ? '병합 재시도' : '다시 시도'}</button> : <span className="platform-media-job-action-space" aria-hidden="true" />}<span>{job.state === 'completed' ? capturePresentation(job.library)?.label ?? mediaStateLabel[job.state] : mediaStateLabel[job.state]}</span></div>
      </div>)}
      {!jobs.some(job => sectionOf(job) === section) ? <p className="platform-media-work-empty">{section === 'queued' ? '대기 중인 작업이 없습니다.' : section === 'active' ? '진행 중인 작업이 없습니다.' : '완료된 작업이 없습니다.'}</p> : null}
    </div>
  </section>;
}
