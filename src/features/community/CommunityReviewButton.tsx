import { useCallback, useId, useRef, useState } from "react";
import { communityApi, type CommunityApi, type WorkKey } from "./api";
import { AlbumCommentsPopover } from "./AlbumCommentsPopover";
import { useAlbumComments } from "./useAlbumComments";

type Props = { work: WorkKey; small?: boolean; api?: CommunityApi };
export function CommunityReviewButton(props: Props) {
  return <WorkCommentButton key={`${props.work.source}:${props.work.workId}`} {...props} />;
}
function WorkCommentButton({ work, small = false, api = communityApi }: Props) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const comments = useAlbumComments(work, open, api);
  const close = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) trigger.current?.focus({ preventScroll: true });
  }, []);

  return (<>
    <button
      ref={trigger}
      type="button"
      className={`icon-button community-review-button${small ? " small" : ""}`}
      aria-label="코멘트 남기기"
      title="코멘트 남기기"
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-controls={open ? id : undefined}
      onClick={() => setOpen((old) => !old)}
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
        <path d="M5 3.5h14a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H9l-5 3v-3a2 2 0 0 1-1-1.73V5.5a2 2 0 0 1 2-2Z" />
        <path d="M7.5 8.5h9m-9 5h6" />
      </svg>
    </button>
    {open ? <AlbumCommentsPopover id={id} trigger={trigger} state={comments} onClose={close} /> : null}
  </>);
}
