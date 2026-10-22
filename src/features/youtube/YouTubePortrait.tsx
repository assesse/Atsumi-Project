import { useEffect, useRef, useState } from 'react';
import type { YouTubeApi } from './api';
import { parseYouTubeTarget, type YouTubeMetadata } from './model';

import { isYouTubeImage as imageData, subscribeYouTubeThumbnail } from './thumbnailRequests';

function channel(item?: YouTubeMetadata) {
  const target = item?.kind === 'channel' ? parseYouTubeTarget(item.url) : item?.authorUrl ? parseYouTubeTarget(item.authorUrl) : null;
  return target?.kind === 'channel' ? target : null;
}

/** Portrait-only source; never treats a video cover as its author's image. */
export function YouTubePortrait({ item, api, active, privacy }: { item?: YouTubeMetadata; api?: YouTubeApi; active: boolean; privacy: boolean }) {
  const target = channel(item), id = target?.id ?? '';
  const initial = item?.kind === 'channel' && imageData(item.thumbnail) ? item.thumbnail : null;
  const root = useRef<HTMLSpanElement>(null);
  const [loaded, setLoaded] = useState<{ id: string; image: string | null } | null>(null);
  const [failed, setFailed] = useState('');
  useEffect(() => {
    setLoaded(null); setFailed('');
    if (!active || privacy || !id || !api?.thumbnail || initial) return;
    let gone = false, release: (() => void) | undefined;
    const load = () => { if (!gone && !release) release = subscribeYouTubeThumbnail(api.thumbnail!, id, image => { if (!gone) setLoaded({ id, image }); }); };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { observer!.disconnect(); load(); }
    }, { rootMargin: '100px' });
    if (observer && root.current) observer.observe(root.current); else load();
    return () => { gone = true; release?.(); observer?.disconnect(); };
  }, [id, initial, api, active, privacy]);
  const image = !privacy ? initial ?? (loaded?.id === id ? loaded.image : null) : null;
  return <span ref={root} className="youtube-channel-avatar" aria-hidden="true">{!privacy && item ? (item.author || item.title).slice(0, 1) : ''}{image && failed !== image ? <img src={image} alt="" decoding="async" onError={() => setFailed(image)} /> : null}</span>;
}
