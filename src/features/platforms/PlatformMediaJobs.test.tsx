import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { PlatformMediaJobs } from './PlatformMediaJobs';
import type { MediaJob } from './platformMedia';

it('animates only actual new settlement, not initial, stale, re-scoped or reduced-motion counts', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'animate');
  const cancel = vi.fn(), animate = vi.fn(() => ({ cancel }));
  Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, value: animate });
  const reduced = vi.fn(() => ({ matches: false })); vi.stubGlobal('matchMedia', reduced);
  const host = document.createElement('div'), root = createRoot(host);
  const render = async (completed: number, stale = false, prefix = '') => {
    const jobs: MediaJob[] = Array.from({ length: 10 }, (_, i) => ({ jobId: `${prefix}${i}`, requestId: `${i}`, reference: { provider: 'youtube', kind: 'video', externalId: 'jNQXAC9IVRw' }, revision: 1, state: i < completed ? 'completed' : 'verifying', title: '', channelName: '', createdAt: 1, receivedBytes: 0 }));
    await act(async () => root.render(<PlatformMediaJobs jobs={jobs} privacyMode={false} stale={stale} busy={false} onCancel={vi.fn()} onRetry={vi.fn()} />));
  };
  try {
    await render(1); expect(animate).not.toHaveBeenCalled();
    expect(host.querySelector('.queue-summary-heading')).toHaveTextContent('최근 작업');
    expect(host.querySelector('.queue-summary-heading')).not.toHaveTextContent('전체 작업');
    await render(2); expect(animate).toHaveBeenCalledTimes(2);
    await render(3, true); expect(cancel).toHaveBeenCalledTimes(2);
    await render(4); await render(5, false, 'new'); expect(animate).toHaveBeenCalledTimes(2);
    reduced.mockReturnValue({ matches: true }); await render(6, false, 'new'); expect(animate).toHaveBeenCalledTimes(2);
    reduced.mockReturnValue({ matches: false }); await render(10, false, 'new'); expect(animate).toHaveBeenCalledTimes(4);
    expect(host.querySelector('.queue-summary')).toHaveClass('is-finished');
    expect(host.querySelector('.queue-summary')).not.toHaveClass('is-running');
  } finally {
    await act(async () => root.unmount());
    if (descriptor) Object.defineProperty(HTMLElement.prototype, 'animate', descriptor);
    else delete (HTMLElement.prototype as Partial<HTMLElement>).animate;
    vi.unstubAllGlobals();
  }
});

it('settles a partial raw recording without a normal-completion animation or retry action', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'animate');
  const animate = vi.fn(() => ({ cancel: vi.fn() }));
  Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, value: animate });
  const host = document.createElement('div'), root = createRoot(host);
  const job: MediaJob = { jobId: 'raw-job', requestId: 'raw-request', reference: { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' }, revision: 1, state: 'verifying', title: 'Fixture', channelName: '', createdAt: 1, receivedBytes: 300 };
  const render = async () => act(async () => root.render(<PlatformMediaJobs jobs={[job]} privacyMode={false} stale={false} busy={false} onCancel={vi.fn()} onRetry={vi.fn()} />));
  try {
    await render();
    job.state = 'completed';
    job.library = { libraryId: 'raw-library', revision: 2, reference: job.reference, title: 'Fixture', channelName: '', completedAt: 1, bytes: 300, duration: 1, sha256: 'a'.repeat(64), captureOutcome: { state: 'partial', reasons: ['incomplete_structure'], excludedUnits: [1, 0], excludedBytes: [1, 0] } };
    await render();
    expect(animate).not.toHaveBeenCalled();
    expect(host.querySelector('.queue-summary')).not.toHaveClass('is-finished');
    expect(host.querySelector('.queue-progress-segment.tone-error')).toHaveStyle({ width: '100%' });
    await act(async () => (host.querySelectorAll('[role="tab"]')[2] as HTMLButtonElement).click());
    expect(host.querySelector('[role="tabpanel"]')).toHaveTextContent('일부 저장');
    expect(host.querySelector('[role="tabpanel"] button')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    if (descriptor) Object.defineProperty(HTMLElement.prototype, 'animate', descriptor);
    else delete (HTMLElement.prototype as Partial<HTMLElement>).animate;
    vi.unstubAllGlobals();
  }
});
