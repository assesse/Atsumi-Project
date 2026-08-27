import { useEffect, useRef, useState } from "react";
import { FluentIcon } from "./FluentIcon";

type TutorialDialogProps = {
  open: boolean;
  onClose: (doNotShowAgain: boolean) => void;
};

const tutorialSteps = [
  { icon: "\uE721", title: "조건을 조합해 탐색", description: "작가와 태그를 함께 검색할 수 있습니다.", example: "artist:healthyman female:ahegao" },
  { icon: "\uE896", title: "앨범 다운로드", description: "앨범 카드를 선택해 다운로드하고 진행 상태를 확인합니다.", example: "Ctrl+Enter · 선택 항목 다운로드" },
  { icon: "\uE8B3", title: "판본과 중복 검토", description: "겹치는 페이지를 비교하고 보존·제거·격리를 결정합니다.", example: "Downloads · 중복 검사" },
  { icon: "\uE765", title: "키보드로 빠르게", description: "검색, 선택, 화면 이동을 단축키로 실행할 수 있습니다.", example: "/ 또는 ? · 단축키 안내" },
] as const;

export function TutorialDialog({ open, onClose }: TutorialDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const startButton = useRef<HTMLButtonElement>(null);
  const closingInternally = useRef(false);
  const [doNotShowAgain, setDoNotShowAgain] = useState(false);

  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (open && !node.open) {
      setDoNotShowAgain(false);
      node.showModal();
      window.requestAnimationFrame(() => startButton.current?.focus());
    } else if (!open && node.open) {
      closingInternally.current = true;
      node.close();
    }
  }, [open]);

  const close = () => onClose(doNotShowAgain);

  return (
    <dialog ref={dialog} className="tutorial-dialog" aria-labelledby="tutorial-title" onCancel={(event) => { event.preventDefault(); close(); }} onClose={() => { if (closingInternally.current) { closingInternally.current = false; return; } close(); }}>
      <div className="tutorial-body">
        <header className="tutorial-header">
          <div><span>WELCOME TO ATSUMI</span><h2 id="tutorial-title">Atsumi 시작하기</h2><p>탐색부터 중복 정리까지 필요한 흐름만 간단히 안내합니다.</p></div>
          <button type="button" className="tutorial-close" aria-label="튜토리얼 닫기" title="닫기" onClick={close}>×</button>
        </header>
        <div className="tutorial-steps">
          {tutorialSteps.map((step, index) => (
            <section key={step.title}>
              <div className="tutorial-step-number">{index + 1}</div><FluentIcon glyph={step.icon} />
              <div><h3>{step.title}</h3><p>{step.description}</p><code>{step.example}</code></div>
            </section>
          ))}
        </div>
        <footer className="tutorial-footer">
          <label><input type="checkbox" checked={doNotShowAgain} onChange={(event) => setDoNotShowAgain(event.target.checked)} /><span>다시 보지 않기</span></label>
          <button ref={startButton} type="button" className="text-button primary" onClick={close}>Atsumi 시작</button>
        </footer>
      </div>
    </dialog>
  );
}
