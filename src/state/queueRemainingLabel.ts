import type { QueueSnapshot } from "../api/workConsole";

/** Reuse the queue's terminal-throughput estimate; never equate received bytes with completion. */
export function queueRemainingLabel(snapshot?: Pick<QueueSnapshot, "globalActive" | "etaSeconds" | "recentCompleted"> | null, error?: string | null): string {
  if (error) return "예상 시간 확인 중";
  if (!snapshot || snapshot.globalActive === 0) return "";
  const seconds = snapshot.etaSeconds;
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0 || snapshot.recentCompleted < 3) return "예상 시간 계산 중";
  // Round up to minutes, or five minutes for long queues. Avoid a ticking clock.
  const minutes = Math.ceil(seconds / (seconds >= 3600 ? 300 : 60)) * (seconds >= 3600 ? 5 : 1);
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return `약 ${hours ? `${hours}시간${remainder ? " " : ""}` : ""}${remainder ? `${remainder}분` : ""} 남음`;
}
