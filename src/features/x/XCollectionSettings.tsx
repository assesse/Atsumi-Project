import { useState } from "react";
import { saveXCollectionCardWidth, useXCollectionCardWidth, X_CARD_WIDTH } from "./collectionPreferences";

export function XCollectionSettings() {
  const width = useXCollectionCardWidth();
  const [failed, setFailed] = useState(false);
  const id = "settings-x-card-width";
  return <div className="x-collection-settings"><div className="setting-row">
    <label htmlFor={id}><strong>카드 크기</strong></label>
    <div className="range-wrap">
      <input id={id} type="range" aria-label="카드 크기" min={X_CARD_WIDTH.min} max={X_CARD_WIDTH.max}
        step={X_CARD_WIDTH.step} value={width} onChange={event => setFailed(!saveXCollectionCardWidth(Number(event.target.value)))} />
      <output htmlFor={id}>{width}px</output>
    </div>
    {failed ? <p className="inline-error" role="alert">카드 크기를 저장하지 못했습니다.</p> : null}
  </div></div>;
}
