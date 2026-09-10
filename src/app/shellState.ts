import type { ContentSource } from "./workspaceRegistry";

/** Only application chrome lives here; feature navigation and jobs do not. */
export type ShellState = {
  source: ContentSource;
  railCollapsed: boolean;
  settingsOpen: boolean;
  activityOpen: boolean;
};

export type ShellAction =
  | { type: "source.select"; source: ContentSource }
  | { type: "rail.toggle" }
  | { type: "settings.set"; open: boolean }
  | { type: "activity.set"; open: boolean };

export const createShellState = (source: ContentSource): ShellState => ({
  source,
  railCollapsed: false,
  settingsOpen: false,
  activityOpen: false,
});

export function shellReducer(state: ShellState, action: ShellAction): ShellState {
  switch (action.type) {
    case "source.select": return state.source === action.source ? state : { ...state, source: action.source };
    case "rail.toggle": return { ...state, railCollapsed: !state.railCollapsed };
    case "settings.set": return { ...state, settingsOpen: action.open };
    case "activity.set": return { ...state, activityOpen: action.open };
  }
}
