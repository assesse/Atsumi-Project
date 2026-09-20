import { useCallback, useEffect, useRef, useState } from "react";
import { communityError, type CommunityApi, type Cursor, type ReviewPage, type WorkKey, type Writer } from "./api";

/** Local to one work/button. Closing the popover keeps its draft, not an auth
 * token. Only an explicit writing gesture asks native code for an identity. */
export function useAlbumComments(work: WorkKey, open: boolean, api: CommunityApi) {
  const [page, setPage] = useState<ReviewPage>({ items: [], nextCursor: null });
  const [shown, setShown] = useState(3);
  const [loading, setLoading] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [writer, setWriter] = useState<Writer | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [writeError, setWriteError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [draft, setDraft] = useState({ rating: 0, comment: "" });
  const [saved, setSaved] = useState(false);
  const mounted = useRef(false), readSequence = useRef(0), readPending = useRef(false), savePending = useRef(false);
  const writerRequest = useRef<Promise<void> | null>(null);
  const dirty = useRef({ rating: false, comment: false });
  const key = useRef(work).current;

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++readSequence.current; }; }, []);
  const load = useCallback(async (cursor: Cursor | null = null) => {
    const sequence = ++readSequence.current;
    readPending.current = true; setLoading(true); setReadError(null);
    try {
      const result = await api.work(key, cursor);
      if (!mounted.current || sequence !== readSequence.current) return;
      setPage((old) => ({ ...result, items: cursor ? [...old.items, ...result.items.filter((item) => !old.items.some((previous) => previous.id === item.id))] : result.items }));
      if (cursor) setShown((old) => old + 3);
    } catch (error) {
      if (mounted.current && sequence === readSequence.current) setReadError(communityError(error));
    } finally {
      if (mounted.current && sequence === readSequence.current) { readPending.current = false; setLoading(false); }
    }
  }, [api, key]);
  useEffect(() => { if (open) { setShown(3); void load(); } }, [open, load]);

  const beginWriting = () => {
    if (writer || writerRequest.current) return;
    setPreparing(true); setWriteError(null);
    writerRequest.current = api.beginWriting(key).then((value) => {
      if (!mounted.current) return;
      setWriter(value);
      // Typing/rating can start before the server replies. Never replace that
      // first input with an older saved review when the response arrives.
      setDraft((old) => ({ rating: dirty.current.rating ? old.rating : value.mine?.rating ?? 0,
        comment: dirty.current.comment ? old.comment : value.mine?.comment ?? "" }));
    }).catch((error) => { if (mounted.current) setWriteError(communityError(error)); })
      .finally(() => { writerRequest.current = null; if (mounted.current) setPreparing(false); });
  };
  const setRating = (rating: number) => {
    dirty.current.rating = true; setDraft((old) => ({ ...old, rating })); setNotice(null); beginWriting();
  };
  const setComment = (comment: string) => {
    dirty.current.comment = true; setDraft((old) => ({ ...old, comment })); setNotice(null); beginWriting();
  };
  const submit = async () => {
    if (savePending.current || !writer || draft.rating < 1 || draft.rating > 5) return;
    savePending.current = true; setSaving(true); setWriteError(null); setNotice(null);
    try {
      await api.save({ ...key, nickname: writer.profile.nickname, rating: draft.rating,
        comment: draft.comment.trim(), recommended: writer.mine?.recommended ?? false });
      if (!mounted.current) return;
      setSaved(true); setNotice(writer.mine?.hidden ? "저장했어요. 운영자가 숨긴 상태는 유지됩니다." : "저장했어요.");
      setShown(3); void load();
    } catch (error) { if (mounted.current) setWriteError(communityError(error)); }
    finally { savePending.current = false; if (mounted.current) setSaving(false); }
  };
  const more = () => {
    if (readPending.current) return;
    if (shown < page.items.length) setShown((old) => old + 3);
    else if (page.nextCursor) void load(page.nextCursor);
  };
  return { page, shown, loading, readError, writer, preparing, saving, writeError, notice, draft, saved,
    beginWriting, setRating, setComment, submit, more, reload: () => load() };
}

export type AlbumCommentsState = ReturnType<typeof useAlbumComments>;
