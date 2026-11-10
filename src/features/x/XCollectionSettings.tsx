import { useState } from "react";
import { saveXCollectionCardWidth, useXCollectionCardWidth, X_CARD_WIDTH, type XCollection } from "./collectionPreferences";

export function XCollectionSettings() {
  return <div className="x-collection-settings">
    <CollectionSize feed="likes" label="마음에 들어요 카드 크기" />
    <CollectionSize feed="bookmarks" label="북마크 카드 크기" />
  </div>;
}
function CollectionSize({ feed, label }: { feed: XCollection; label: string }) {
  const width = useXCollectionCardWidth(feed);
  const [failed, setFailed] = useState(false);
  const id = "settings-x-" + feed + "-card-width";
  return <div className="setting-row">
    <label htmlFor={id}><strong>{label}</strong></label>
    <div className="range-wrap">
      <input id={id} type="range" aria-label={label} min={X_CARD_WIDTH.min} max={X_CARD_WIDTH.max}
        step={X_CARD_WIDTH.step} value={width} onChange={event => setFailed(!saveXCollectionCardWidth(feed, Number(event.target.value)))} />
      <output htmlFor={id}>{width}px</output>
    </div>
    {failed ? <p className="inline-error" role="alert">카드 크기를 저장하지 못했습니다.</p> : null}
  </div>;
}
