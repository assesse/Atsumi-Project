import { useEffect, useRef, useState } from "react";
import { GalleryThumbnail } from "../../components/GalleryThumbnail";
import { galleryId } from "../../core/types";
import type { WorkKey } from "./api";
import { loadDanbooruPreview } from "./workPreview";

export function CommunityWorkPreview({ work, privacyMode = false }: { work: WorkKey; privacyMode?: boolean }) {
  if (privacyMode) return <div className="community-work-preview is-private">프라이버시 모드</div>;
  const id = Number(work.workId);
  if (!Number.isSafeInteger(id) || id <= 0) return <div className="community-work-preview">미리보기 없음</div>;
  if (work.source === "hitomi") return <GalleryThumbnail className="community-work-preview" thumbnailKey={{ kind: "gallery-cover", galleryId: galleryId(id) }} consumer="review" priority="visible" alt={`Hitomi #${work.workId} 미리보기`} />;
  return <DanbooruPreview key={work.workId} workId={work.workId} />;
}

function DanbooruPreview({ workId }: { workId: string }) {
  const root = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState<string | null | undefined>();
  const [loaded, setLoaded] = useState(false);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) { setVisible(true); observer.disconnect(); }
    }, { rootMargin: "200px" });
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!visible) return;
    let active = true;
    void loadDanbooruPreview(workId).then((value) => { if (active) setUrl(value); });
    return () => { active = false; };
  }, [visible, workId]);
  return <div ref={root} className={`community-work-preview${url !== null && !loaded ? " is-loading" : ""}`} aria-busy={visible && url !== null && !loaded}>
    {url ? <img src={url} alt={`Danbooru #${workId} 미리보기`} loading="lazy" decoding="async" referrerPolicy="no-referrer" onLoad={() => setLoaded(true)} onError={() => setUrl(null)} /> : url === null ? <span>미리보기 없음</span> : <span className="sr-only">미리보기 불러오는 중</span>}
  </div>;
}
