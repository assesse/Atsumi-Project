import type { ApiError, GalleryPage, SearchSubmission } from "../api/contracts";

export type GalleryQueryPhase = "idle" | "submitting" | "loading-page" | "ready" | "error";

export type GalleryQueryState = {
  phase: GalleryQueryPhase;
  submitToken: number;
  queryId: string | null;
  page: GalleryPage | null;
  pendingPage: number | null;
  error: ApiError | null;
};

export const initialGalleryQueryState: GalleryQueryState = {
  phase: "idle",
  submitToken: 0,
  queryId: null,
  page: null,
  pendingPage: null,
  error: null,
};

export type GalleryQueryAction =
  | { type: "submit.started"; token: number }
  | { type: "submit.succeeded"; token: number; submission: SearchSubmission }
  | { type: "submit.failed"; token: number; error: ApiError }
  | { type: "page.started"; queryId: string; page: number }
  | { type: "page.succeeded"; queryId: string; page: GalleryPage }
  | { type: "page.failed"; queryId: string; page: number; error: ApiError }
  | { type: "reset" };

export function galleryQueryReducer(
  state: GalleryQueryState,
  action: GalleryQueryAction,
): GalleryQueryState {
  switch (action.type) {
    case "submit.started":
      return {
        phase: "submitting",
        submitToken: action.token,
        queryId: null,
        page: null,
        pendingPage: null,
        error: null,
      };
    case "submit.succeeded":
      if (action.token !== state.submitToken || state.phase !== "submitting") return state;
      return {
        ...state,
        phase: "ready",
        queryId: action.submission.queryId,
        page: action.submission.firstPage,
        error: null,
      };
    case "submit.failed":
      if (action.token !== state.submitToken || state.phase !== "submitting") return state;
      return { ...state, phase: "error", error: action.error };
    case "page.started":
      if (action.queryId !== state.queryId) return state;
      return { ...state, phase: "loading-page", pendingPage: action.page, error: null };
    case "page.succeeded":
      if (
        action.queryId !== state.queryId ||
        state.phase !== "loading-page" ||
        action.page.page !== state.pendingPage
      ) return state;
      return { ...state, phase: "ready", page: action.page, pendingPage: null, error: null };
    case "page.failed":
      if (
        action.queryId !== state.queryId ||
        state.phase !== "loading-page" ||
        action.page !== state.pendingPage
      ) return state;
      return { ...state, phase: "error", pendingPage: null, error: action.error };
    case "reset":
      return { ...initialGalleryQueryState, submitToken: state.submitToken + 1 };
  }
}
