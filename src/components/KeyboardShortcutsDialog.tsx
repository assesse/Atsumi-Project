import { useEffect, useRef } from "react";

type KeyboardShortcutsDialogProps = {
  open: boolean;
  onClose: () => void;
};

const shortcutGroups = [
  {
    title: "선택과 이동",
    shortcuts: [
      ["Ctrl+A", "현재 결과 전체 선택"],
      ["Ctrl+Shift+A", "선택 해제"],
      ["← ↑ ↓ →", "카드 포커스 이동"],
      ["Shift+방향키", "범위 선택"],
      ["Space", "포커스한 카드 선택 전환"],
      ["Ctrl+카드 클릭 · 체크박스", "카드 다중선택 전환"],
      ["Shift+클릭", "범위 선택"],
    ],
  },
  {
    title: "열기와 작업",
    shortcuts: [
      ["Enter", "다중선택한 앨범을 각각 상세 탭으로 열기 · 단일 카드는 상세/완료 폴더"],
      ["Ctrl+Enter", "선택 항목 다운로드 또는 재시도"],
      ["Delete", "Explore·Auto Find 탐색 제외 또는 Downloads 격리"],
      ["Ctrl+작가·태그 클릭", "검색을 백그라운드 탭에 열기"],
      ["Ctrl+관련 갤러리 클릭", "관련 앨범을 백그라운드 탭에 열기"],
      ["Ctrl+Z", "최근 닫기 또는 제외·격리 실행 취소 (입력창 제외)"],
      ["Ctrl+Shift+T", "최근 닫은 상세·검색 탭 복원"],
    ],
  },
  {
    title: "Floating Detail",
    shortcuts: [
      ["Q · E", "이전·다음 상세 탭"],
      ["A · D · ← · →", "추가 미리보기 이전·다음 묶음"],
      ["A · D · ← · →", "PAGE PREVIEW 이전·다음 페이지"],
    ],
  },
  {
    title: "앱 이동",
    shortcuts: [
      ["Ctrl+F", "현재 화면 검색으로 이동"],
      ["F5", "현재 결과 새로고침"],
      ["Ctrl+Tab", "다음 화면"],
      ["Ctrl+Shift+Tab", "이전 화면"],
      ["Esc", "맨 위 화면 닫기 → 출발 화면 복귀 (숨겨진 상세는 유지)"],
      ["? · /", "이 도움말"],
    ],
  },
] as const;

export function KeyboardShortcutsDialog({ open, onClose }: KeyboardShortcutsDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const closingInternally = useRef(false);

  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (open && !node.open) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      node.showModal();
      window.requestAnimationFrame(() => closeButton.current?.focus());
    } else if (!open && node.open) {
      closingInternally.current = true;
      node.close();
      const target = opener.current;
      opener.current = null;
      window.requestAnimationFrame(() => {
        if (target?.isConnected) target.focus();
        else document.querySelector<HTMLElement>('.view-header input[aria-label="검색"]')?.focus();
      });
    }
  }, [open]);

  return (
    <dialog
      ref={dialog}
      className="keyboard-shortcuts-dialog"
      aria-labelledby="keyboard-shortcuts-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={() => {
        if (closingInternally.current) {
          closingInternally.current = false;
          return;
        }
        onClose();
      }}
    >
      <div className="keyboard-shortcuts-body">
        <div className="keyboard-shortcuts-header">
          <div>
            <span>KEYBOARD</span>
            <h2 id="keyboard-shortcuts-title">키보드 단축키</h2>
          </div>
          <button ref={closeButton} type="button" aria-label="단축키 도움말 닫기" title="닫기" onClick={onClose}>×</button>
        </div>
        <div className="keyboard-shortcuts-groups">
          {shortcutGroups.map((group) => (
            <section key={group.title}>
              <h3>{group.title}</h3>
              <dl>
                {group.shortcuts.map(([keys, description]) => (
                  <div key={`${keys}:${description}`}>
                    <dt>{keys.split("+").map((key) => <kbd key={key}>{key}</kbd>)}</dt>
                    <dd>{description}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
        <p className="keyboard-shortcuts-note">입력창과 열린 대화상자에서는 편집 키를 가로채지 않습니다.</p>
      </div>
    </dialog>
  );
}
