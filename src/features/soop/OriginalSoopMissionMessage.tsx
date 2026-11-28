import { useEffect, useState } from 'react';
import type { MediaReplayChatItem, SoopMissionMetadata } from '../platforms/platformMedia';
import { safeChatImage } from '../platforms/PlatformChatParts';
import { soopMissionAssetIds, soopMissionNotice } from './soopMission';

/** Original six service121 branches; local recorded assets, never a live payment action. */
export function OriginalSoopMissionMessage({ row, images, showTimes, onProfile }: {
  row: MediaReplayChatItem & { soopMission: SoopMissionMetadata };
  images: ReadonlyMap<string, string>; showTimes: boolean; onProfile(): void;
}) {
  const m = row.soopMission, name = m.kind === 'battle' ? '대결미션' : '도전미션';
  const id = soopMissionAssetIds(row)[0], local = id ? images.get(id) : undefined;
  const [failed, setFailed] = useState<string>();
  useEffect(() => setFailed(undefined), [local]);
  const image = safeChatImage(local) && local !== failed ? local : undefined;
  const seconds = Math.floor(Math.max(0, row.timeMs ?? 0) / 1000);
  const stamp = showTimes && row.timeMs !== null ? <time className="soop-replay-time">{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, '0')}</time> : null;
  const author = () => <button type="button" className="name platform-replay-chat-author" onClick={onProfile}>{row.author}</button>;
  if (m.event === 'notice') return <div className="chatting-list-item platform-replay-chat-row" data-chat-row-id={row.id}>
    {stamp}<div className="bubble-container"><strong className="title"><i className="ic-notice" />{name}</strong><p className="msg">{soopMissionNotice(m)}</p></div>
  </div>;
  return <>
    <div className="chatting-list-item platform-replay-chat-row" data-chat-row-id={row.id}>
      {stamp}<div className={`donation-container${m.event === 'gift' ? ' basic' : ''}`}><div className="donation-bubble">
        <div className="ceremony-img custom-mobile-img">
          {image ? <img src={image} alt={m.kind === 'battle' ? '대결미션 별풍선 이미지' : m.event === 'gift' ? '도전미션 별풍선 후원 이미지' : '도전미션 별풍선 이미지'} onError={() => setFailed(image)} /> : null}
        </div>
        <div className="info-box">
          {m.event === 'gift' ? author() : null}
          <span className="money">{m.event === 'gift' ? <>{m.relay ? m.kind === 'battle' ? <span className="relay">중계방 </span> : '중계방 ' : null}{name} 별풍선<br /><em>{m.count}</em>개 후원{m.kind === 'battle' ? '!' : ''}</>
            : <>{name}<br />별풍선 <em>{m.count}</em>개 획득!</>}</span>
          {!image ? <small className="soop-replay-time" title="당시 미션 이미지를 읽을 수 없습니다. 저장된 내용은 그대로 표시합니다.">이미지 없음</small> : null}
        </div>
      </div></div>
    </div>
    {m.kind === 'battle' && m.event === 'gift' && (m.fanOrder ?? 0) > 0 ? <div className="chatting-list-item" user-type="fan"><div className="system-message donation-state"><p className="msg">{author()}님이 {m.fanOrder}번째 팬클럽이 되셨습니다.</p></div></div> : null}
    {m.kind === 'battle' && m.event === 'gift' && m.topFan === true ? <div className="chatting-list-item" user-type="vip"><div className="system-message donation-state"><p className="msg">{author()}님이 열혈팬이 되셨습니다.</p></div></div> : null}
  </>;
}
