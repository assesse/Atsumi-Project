import { createContext, useContext } from "react";
import type { ContentSource, WorkspaceViewId } from "./workspaceRegistry";
import type { WorkKey } from "../features/community/api";

export type NavigationRequest = { source: ContentSource; view: WorkspaceViewId; sequence: number };
export const CommonNavigationContext = createContext<{
  communityOpen: boolean;
  /** A work key is supplied only for an explicit review-writing action. */
  openCommunity: (reviewWork?: WorkKey) => void;
} | null>(null);
export const useCommonNavigation = () => useContext(CommonNavigationContext);
