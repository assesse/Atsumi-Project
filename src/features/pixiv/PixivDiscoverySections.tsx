import type { ReactNode } from "react";
import type { PixivDiscovery, PixivWork } from "./model";
import { PixivMetadataText } from "./PixivMetadataText";
import "./PixivDiscoverySections.css";

export const discoveryWorks = (value: PixivDiscovery) => [...new Map([
  ...value.sections.flatMap(section => section.items), ...value.creators.flatMap(creator => creator.works),
].map(work => [work.id, work])).values()];
export function appendDiscovery(current: PixivDiscovery, next: PixivDiscovery): PixivDiscovery {
  const sections = current.sections.map(section => {
    const added = next.sections.find(row => row.id === section.id);
    return added ? { ...section, items: [...new Map([...section.items, ...added.items].map(work => [work.id, work])).values()] } : section;
  });
  return { ...current, sections: [...sections, ...next.sections.filter(section => !sections.some(row => row.id === section.id))], cursor: next.cursor, page: next.page, issues: [...new Set([...current.issues, ...next.issues])] };
}

export function PixivDiscoverySections({ value, privacyMode, renderWork, renderImage, onTag, onCreator }: {
  value: PixivDiscovery; privacyMode: boolean;
  renderWork: (work: PixivWork) => ReactNode;
  renderImage: (url: string | undefined, name: string) => ReactNode;
  onTag: (tag: string) => void;
  onCreator: (id: string, name: string, background: boolean) => void;
}) {
  return <div className="pixiv-discovery-sections">
    {value.issues.length > 0 && <div className="pixiv-discovery-issues" role="status">{value.issues.map(issue => <p key={issue}>{issue}</p>)}</div>}
    {value.sections.map(section => <section className="pixiv-discovery-section" key={section.id} aria-label={privacyMode ? "작품 목록" : section.title}>
      <header><h2>{privacyMode ? "작품 목록" : section.tag ? <PixivMetadataText text={section.tag} kind="tag" hints={value.suggestions?.tagTranslations}>{section.title}</PixivMetadataText> : section.title}</h2>{section.tag && !privacyMode && <button type="button" onClick={() => onTag(section.tag!)}>더 보기</button>}</header>
      {section.items.length > 0 ? <div className="pixiv-grid">{section.items.map(renderWork)}</div>
        : <p className="pixiv-section-empty">{section.kind === "followed" ? "이 분류에 표시할 팔로우 신작이 없습니다." : "표시할 작품이 없습니다."}</p>}
    </section>)}
    {value.creators.length > 0 && <section className="pixiv-discovery-section pixiv-recommended-creators" aria-label="추천 작가">
      <header><h2>추천 작가</h2></header>
      <div className="pixiv-creator-grid">{value.creators.map(creator => <article className="pixiv-creator" key={creator.id}>
        <PixivMetadataText as="button" text={creator.name} kind="artist" private={privacyMode} className="pixiv-creator-heading" onClick={event => onCreator(creator.id, creator.name, event.ctrlKey || event.metaKey)}>
          {renderImage(creator.imageUrl, creator.name)}<strong>{privacyMode ? "작가" : creator.name}</strong>
          <span aria-hidden="true">›</span>
        </PixivMetadataText>
        <div className="pixiv-creator-works">{creator.works.map(renderWork)}</div>
      </article>)}</div>
    </section>}
  </div>;
}
