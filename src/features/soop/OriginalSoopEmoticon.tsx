import { useState } from 'react';
import type { SoopEmoticonMetadata } from '../platforms/platformMedia';
import { safeChatImage } from '../platforms/PlatformChatParts';
import { soopEmoticonAssetIds } from './soopEmoticon';

/** Original LIVE emote block, backed exclusively by captured local raster assets. */
export function OriginalSoopEmoticon({ value, images, show, small, animate, theme = 'dark' }: {
  value: SoopEmoticonMetadata; images: ReadonlyMap<string, string>; show: boolean; small: boolean; animate: boolean; theme?: 'light' | 'dark';
}) {
  const placeholder = `/assets/soop/ogq_default${theme === 'dark' ? '_dark' : ''}.svg`;
  const valid = new Set(soopEmoticonAssetIds(value));
  const staticId = small ? value.smallStaticImageAssetId : value.staticImageAssetId;
  const animationId = small ? value.smallAnimatedImageAssetId : value.animatedImageAssetId;
  const requestedAnimation = value.animated && animate;
  const candidates = [
    { id: requestedAnimation ? animationId : null, kind: 'animated' },
    { id: staticId, kind: 'static' },
  ].flatMap(({ id, kind }) => typeof id === 'string' && valid.has(id) && safeChatImage(images.get(id)) ? [{ url: images.get(id)!, kind }] : []);
  candidates.push({ url: placeholder, kind: 'placeholder' });
  const key = `${small}:${animate}:${animationId}:${staticId}:${theme}`;
  const [failed, setFailed] = useState<{ key: string; urls: string[] }>({ key: '', urls: [] });
  const selected = candidates.find(({ url }) => failed.key !== key || !failed.urls.includes(url));
  const label = value.kind === 'gem' ? 'Gem 이모티콘' : 'OGQ 이모티콘';
  if (!show || !selected) return <div className="msg">/{label}/<br /></div>;
  const staticFallback = requestedAnimation && selected.kind === 'static';
  return <div className={`emoticon-box${small ? ' small' : ''}`}><a className="img-box">
    <img className="emoticon" src={selected.url} alt={staticFallback ? `${label} (정지 이미지)` : label} draggable={false}
      data-image-state={staticFallback ? 'static-fallback' : selected.kind}
      title={selected.kind === 'placeholder' ? '저장된 이모티콘 이미지를 불러오지 못했습니다.' : staticFallback ? '움직이는 이미지 대신 저장된 정지 이미지로 표시합니다.' : undefined}
      onError={() => setFailed(previous => ({ key, urls: [...(previous.key === key ? previous.urls : []), selected.url] }))} />
  </a></div>;
}
