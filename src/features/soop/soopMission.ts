import type { MediaReplayChatItem, SoopMissionMetadata } from '../platforms/platformMedia';
const count = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 0xffffffff;
const text = (v: unknown, limit: number): v is string => typeof v === 'string' && v.length > 0 && [...v].length <= limit && !/[\u0000-\u001f\u007f-\u009f]/.test(v);
const absent = (v: unknown) => v === null || v === undefined;

export function validSoopMission(value: unknown): value is SoopMissionMetadata {
  if (!value || typeof value !== 'object') return false;
  const m = value as SoopMissionMetadata;
  if (m.kind !== 'battle' && m.kind !== 'challenge') return false;
  if (m.event === 'gift') return count(m.count) && m.count > 0 && typeof m.relay === 'boolean' && absent(m.notice)
    && (absent(m.fanOrder) || count(m.fanOrder)) && (absent(m.topFan) || typeof m.topFan === 'boolean')
    && (m.kind === 'battle' || absent(m.fanOrder) && absent(m.topFan));
  if (m.event === 'settle') return count(m.count) && absent(m.relay) && absent(m.notice) && absent(m.fanOrder) && absent(m.topFan);
  if (m.event !== 'notice' || !absent(m.count) || !absent(m.relay) || !absent(m.fanOrder) || !absent(m.topFan) || !absent(m.imageAssetId)) return false;
  const n = m.notice;
  if (!n || typeof n !== 'object') return false;
  if (m.kind === 'challenge') return n.result === 'challenge' && text(n.title, 512) && typeof n.successful === 'boolean';
  return n.result === 'draw' || n.result === 'winner' && text(n.winner, 160)
    || n.result === 'rank' && count(n.rank) && n.rank > 0 && text(n.team, 160);
}
export function isSoopMissionRow(row: Pick<MediaReplayChatItem, 'kind' | 'soopMission'>): row is MediaReplayChatItem & { soopMission: SoopMissionMetadata } {
  return validSoopMission(row.soopMission) && row.kind === (row.soopMission.event === 'gift' ? 'donation' : 'system');
}
export function soopMissionAssetIds(row: Pick<MediaReplayChatItem, 'kind' | 'soopMission'>): string[] {
  const id = row.soopMission?.imageAssetId;
  return isSoopMissionRow(row) && row.soopMission.event !== 'notice' && typeof id === 'string' && /^[a-f0-9]{64}$/.test(id) ? [id] : [];
}
export function soopMissionNotice(m: SoopMissionMetadata): string {
  const n = m.notice;
  if (!validSoopMission(m) || m.event !== 'notice' || !n) return '';
  switch (n.result) {
    case 'draw': return '대결미션 결과 무승부입니다! 참여한 스트리머에게 수고의 별풍선을 선물해 보세요.';
    case 'winner': return `대결미션 승리 팀은 ${n.winner}입니다. 승리 팀의 스트리머에게 축하의 별풍선을 선물해 보세요.`;
    case 'rank': return `대결미션 ${n.rank}등은 ${n.team}입니다. ${n.rank}등 팀의 스트리머에게 수고의 별풍선을 선물해 보세요.`;
    case 'challenge': return n.successful ? `[${n.title}] 미션을 성공하였습니다! 스트리머에게 축하의 별풍선을 선물해보세요!`
      : `[${n.title}] 미션을 실패하였습니다... 스트리머에게 위로의 별풍선을 선물해보세요!`;
  }
}
