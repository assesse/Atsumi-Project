import { expect, it } from 'vitest';
import { capturePresentation, type MediaCaptureOutcome } from './mediaCaptureOutcome';
import type { MediaLibraryItem } from './platformMedia';
const item = (captureOutcome?: MediaCaptureOutcome | null): MediaLibraryItem => ({
  libraryId: 'sample', revision: 1, reference: { provider: 'youtube', kind: 'live', externalId: 'abcdefghijk' },
  title: 'Sample', channelName: 'Channel', bytes: 10, duration: 1, sha256: 'a'.repeat(64), completedAt: 1, captureOutcome,
});
const normal: MediaCaptureOutcome = { state: 'complete', reasons: [], excludedUnits: [0, 1], excludedBytes: [0, 100] };
it('keeps legacy/provider/VOD neutral and normal AAC boundary exclusion normal', () => {
  expect(capturePresentation(item())).toBeNull(); expect(capturePresentation(item(null))).toBeNull();
  expect(capturePresentation({ ...item(normal), reference: { provider: 'soop', kind: 'live', externalId: 'a' } })).toBeNull();
  expect(capturePresentation({ ...item(normal), reference: { provider: 'youtube', kind: 'video', externalId: 'abcdefghijk' } })).toBeNull();
  expect(capturePresentation(item(normal))).toMatchObject({ label: '정상', normal: true, tone: 'success' });
});
it.each([
  { state: 'partial', reasons: ['incomplete_structure'], label: '일부 저장' },
  { state: 'interrupted', reasons: ['source_interrupted'], label: '중단됨' },
] as const)('keeps $state separate from playable file completion', ({ state, reasons, label }) => {
  expect(capturePresentation(item({ state, reasons: [...reasons], excludedUnits: [1, 0], excludedBytes: [100, 0] })))
    .toMatchObject({ label, normal: false, tone: 'warning' });
});
it.each([
  {}, { ...normal, state: 'unknown' }, { ...normal, reasons: ['arbitrary'] },
  { ...normal, excludedUnits: [1, 0] }, { ...normal, excludedBytes: [-1, 0] },
  { ...normal, excludedBytes: [NaN, 0] }, { ...normal, excludedBytes: [0, 129 * 1024 ** 3] },
  { ...normal, reasons: ['limit'], state: 'partial' },
  { ...normal, reasons: ['limit', 'limit'], state: 'interrupted' },
])('never reports malformed runtime outcome as normal: %j', value => {
  expect(capturePresentation(item(value as MediaCaptureOutcome))).toMatchObject({ label: '확인 필요', normal: false });
});
