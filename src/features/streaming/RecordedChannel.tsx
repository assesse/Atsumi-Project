import { useEffect, useState } from "react";
import { openRecordingChannel, recordingProfile, type RecordingProfile } from "../../api/recordingProfile";
import "./RecordedChannel.css";

export function RecordedChannel({ id, name, image, privacy = false }: { id: string; name?: string | null; image?: string | null; privacy?: boolean }) {
  const [profile, setProfile] = useState<RecordingProfile>({ name: name ?? null, image: image ?? null });
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    setProfile({ name: name ?? null, image: image ?? null }); setError("");
    if (!privacy && (!name || !image)) void recordingProfile(id).then(value => { if (!cancelled) setProfile(value); });
    return () => { cancelled = true; };
  }, [id, name, image, privacy]);
  if (privacy) return null;
  const candidate = profile.image || image;
  const archivedImage = typeof candidate === "string" && candidate.length <= 1_500_000 && /^data:image\/(?:png|jpeg|gif|webp);base64,[a-zA-Z0-9+/]+={0,2}$/.test(candidate) ? candidate : null;
  return <div className="recorded-channel"><button type="button" title="녹화 당시 채널 · 클릭하면 채널 페이지 열기" onClick={event => {
    event.stopPropagation(); void openRecordingChannel(id).catch(() => setError("채널 페이지를 열지 못했습니다."));
  }} aria-label={`${profile.name || name || "녹화 채널"} 채널 페이지 열기`}>
    {archivedImage ? <img src={archivedImage} alt="녹화 당시 채널 프로필" /> : <span className="recorded-channel-placeholder" aria-hidden>◯</span>}
    <span>{profile.name || name || "녹화 채널"}<small>{archivedImage ? "녹화 당시 프로필" : "당시 프로필 이미지 없음"}</small></span>
  </button>{error ? <small role="alert">{error}</small> : null}</div>;
}
