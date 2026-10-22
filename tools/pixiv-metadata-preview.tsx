// Entirely synthetic, offline fixture. Never imports the native API or an account.
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { PixivMetadataText } from "../src/features/pixiv/PixivMetadataText";
import "../src/features/pixiv/PixivWorkspace.css";
import "./pixiv-metadata-preview.css";

function Preview() {
  const [hidden, setHidden] = useState(false), [click, setClick] = useState("");
  return <main className="pixiv-workspace" data-fixture-ready>
    <h1>Pixiv 메타데이터 · 오프라인 검증</h1><p>실제 계정·작품·네트워크 검색을 사용하지 않는 예시입니다.</p>
    <div className="samples">
      <PixivMetadataText as="button" id="tag" text="風景" kind="tag" hints={[{ tag: "風景", ko: "풍경", en: "landscape", romaji: "fuukei" }]} onClick={event => setClick(`風景:${event.ctrlKey}`)} />
      <PixivMetadataText as="button" id="kana" text="ミドリ" kind="artist" />
      <PixivMetadataText as="button" id="artist" text="山田太郎" kind="artist" private={hidden} />
      <PixivMetadataText as="button" id="group" text="ミドリ工房" kind="group" private={hidden} />
      <PixivMetadataText as="button" id="title" text="青い空と猫の休日" kind="title" private={hidden} />
      <PixivMetadataText as="button" id="unknown" text="𠮷𠮷" kind="artist" />
      <PixivMetadataText as="button" id="long" text={"長い作品名と青い空の風景 ".repeat(20).trim()} kind="title" />
      <button id="privacy" type="button" onClick={() => setHidden(value => !value)}>격리 테스트: 가리기</button>
    </div><output id="click-result">{click}</output>
    <PixivMetadataText as="button" id="corner" text="風景" kind="tag" />
  </main>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
