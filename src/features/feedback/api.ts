import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ContentSource } from "../../app/workspaceRegistry";

export const OPEN_FEEDBACK = "atsumi:open-feedback";
export type FeedbackDraft = { id: string; eventCount: number; bytes: number; createdAt: number; payload: unknown };
export interface FeedbackApi {
  prepare(source: ContentSource): Promise<FeedbackDraft>;
  send(draftId: string, comment: string): Promise<{ receiptId: string }>;
}
export const feedbackApi: FeedbackApi = {
  prepare: source => isTauri() ? invoke("feedback_prepare", { source }) : Promise.reject(new Error("Atsumi 앱에서 진단 기록을 보낼 수 있습니다.")),
  send: (draftId, comment) => invoke("feedback_send", { draftId, comment }),
};
export const feedbackError = (error: unknown) => typeof error === "string" ? error : error instanceof Error ? error.message : "전송하지 못했습니다. 잠시 후 다시 시도해 주세요.";
