import type { MediaLibraryItem } from './platformMedia';

const reasons = {
  source_interrupted: '녹화가 중단됐습니다.', cancelled: '녹화가 취소됐습니다.', limit: '녹화 한도에 도달했습니다.',
  format_boundary: '영상 형식이 변경됐습니다.', torn_journal: '마지막 저장 기록이 불완전합니다.',
  uncommitted_bytes: '저장이 확인되지 않은 부분을 제외했습니다.', incomplete_append: '마지막 전송이 완전히 끝나지 않았습니다.',
  incomplete_structure: '불완전한 끝부분을 제외했습니다.', startup_coverage: '시작 부분 일부를 제외했습니다.',
  audio_coverage_tail: '음성이 없는 끝부분을 제외했습니다.',
} as const;
export interface MediaCaptureOutcome {
  state: 'complete' | 'partial' | 'interrupted';
  reasons: (keyof typeof reasons)[];
  excludedUnits: [number, number]; excludedBytes: [number, number];
}
export interface CapturePresentation { label: string; tone: 'success' | 'warning'; detail: string; normal: boolean }
const invalid: CapturePresentation = { label: '확인 필요', tone: 'warning', detail: '녹화 종료 정보를 확인하지 못했습니다.', normal: false };
function valid(value: unknown): value is MediaCaptureOutcome {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (Object.keys(v).some(key => !['state', 'reasons', 'excludedUnits', 'excludedBytes'].includes(key))
      || !Array.isArray(v.reasons) || v.reasons.length > 10 || new Set(v.reasons).size !== v.reasons.length
      || !v.reasons.every(reason => typeof reason === 'string' && Object.hasOwn(reasons, reason))) return false;
  const pair = (data: unknown, max: number): data is [number, number] => Array.isArray(data) && data.length === 2
    && data.every(n => Number.isSafeInteger(n) && n >= 0 && n <= max);
  if (!pair(v.excludedUnits, 100_000) || !pair(v.excludedBytes, 128 * 1024 ** 3)
      || v.excludedBytes[0] + v.excludedBytes[1] > 128 * 1024 ** 3
      || v.excludedUnits.some((n, i) => n !== 0 && (v.excludedBytes as number[])[i] === 0)) return false;
  const interrupted = v.reasons.some(reason => ['source_interrupted', 'cancelled', 'limit'].includes(reason));
  return v.state === 'complete' ? v.reasons.length === 0 && v.excludedUnits[0] === 0 && v.excludedBytes[0] === 0
    : v.state === 'partial' ? !interrupted && v.reasons.length > 0
      : v.state === 'interrupted' && interrupted;
}
/** This status is independent of video publication and chat completeness. */
export function capturePresentation(item?: MediaLibraryItem | null): CapturePresentation | null {
  if (item?.reference.provider !== 'youtube' || item.reference.kind !== 'live' || item.captureOutcome == null) return null;
  if (!valid(item.captureOutcome)) return invalid;
  const outcome = item.captureOutcome;
  if (outcome.state === 'complete') return { label: '정상', tone: 'success', detail: '녹화 영상 검증 완료', normal: true };
  return { label: outcome.state === 'partial' ? '일부 저장' : '중단됨', tone: 'warning', normal: false,
    detail: outcome.reasons.map(reason => reasons[reason]).join(' ') };
}
