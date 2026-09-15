export type WorkspaceBadgeKey = "autoFindCount" | "attentionCount";

export type WorkspaceNavigationItem<View extends string = string> = {
  readonly view: View;
  readonly label: string;
  readonly icon: string;
  readonly badgeKey?: WorkspaceBadgeKey;
  readonly badgeWarning?: boolean;
};

type WorkspaceDefinition = {
  readonly id: string;
  readonly label: string;
  readonly subtitle: string;
  readonly description: string;
  readonly navigation: readonly WorkspaceNavigationItem[];
};

export const workspaceRegistry = {
  hitomi: {
    id: "hitomi",
    label: "Hitomi",
    subtitle: "Hitomi library",
    description: "앨범 탐색·다운로드·중복 검토",
    navigation: [
      { view: "explore", label: "Explore", icon: "\uE80F" },
      { view: "auto-find", label: "Auto Find", icon: "\uE735", badgeKey: "autoFindCount" },
      { view: "downloads", label: "Downloads", icon: "\uE896", badgeKey: "attentionCount", badgeWarning: true },
    ],
  },
  danbooru: {
    id: "danbooru",
    label: "Danbooru",
    subtitle: "Danbooru posts",
    description: "post 검색·미리보기·원본 보관",
    navigation: [
      { view: "explore", label: "Explore", icon: "\uE80F" },
      { view: "downloads", label: "Downloads", icon: "\uE896", badgeKey: "attentionCount", badgeWarning: true },
    ],
  },
  chzzk: {
    id: "chzzk",
    label: "CHZZK",
    subtitle: "CHZZK live recorder",
    description: "공개 라이브 시청·녹화·채팅 보관",
    navigation: [
      { view: "live", label: "라이브", icon: "\uE714" },
      { view: "recordings", label: "녹화 목록", icon: "\uE8B7" },
      { view: "auto-record", label: "자동 녹화", icon: "\uE823" },
    ],
  },
} as const satisfies Record<string, WorkspaceDefinition>;

export type ContentSource = keyof typeof workspaceRegistry;
export type WorkspaceViewId<Source extends ContentSource = ContentSource> =
  (typeof workspaceRegistry)[Source]["navigation"][number]["view"];

export const workspaces = Object.values(workspaceRegistry);

export const isContentSource = (value: unknown): value is ContentSource =>
  typeof value === "string" && Object.prototype.hasOwnProperty.call(workspaceRegistry, value);
