import type { ContentSource } from "../app/workspaceRegistry";

export type TourStep = {
  id: string;
  section: "Hitomi" | "Danbooru" | "CHZZK" | "공통";
  target: string;
  title: string;
  description: string;
  action?: "click" | "dblclick" | "contextmenu";
  /** Only these descendants may perform the action; other controls stay blocked. */
  actionSelector?: string;
  spotlightSelector?: string;
  waitForResult?: boolean;
  /** Existing registrations may be acknowledged, but must never be toggled off. */
  satisfiedSelector?: string;
  satisfiedHint?: string;
  hint?: string;
  emptyActionFallback?: { step: string; message: string; label: string };
};

const followTarget = 'button[data-metadata-namespace="artist"], button[data-metadata-namespace="group"]';
/** Navigation is required. Only explicit favorite registration and Auto Find refresh write data. */
export const hitomiTourSteps: readonly TourStep[] = [
  { id: "settings", section: "공통", target: '[data-tour="hitomi-settings"]', title: "저장 위치 설정", description: "앨범·이미지·녹화의 저장 위치", action: "click", hint: "설정 클릭" },
  { id: "folder", section: "공통", target: '[data-tour="download-folder"]', title: "다운로드 폴더", description: "앨범·이미지·녹화의 공통 저장 위치. 폴더 선택 시 자동 적용." },
  { id: "privacy", section: "공통", target: '[data-tour="privacy-mode"]', title: "프라이버시 모드", description: "기본값은 켜짐. 끄면 이미지 미리보기가 표시됩니다." },
  { id: "search", section: "Hitomi", target: '[data-tour="hitomi-search"]', title: "작가 · 태그 · 작품번호 검색", description: "빈 검색은 전체 탐색. 결과는 탐색 탭별로 유지.", action: "click", hint: "검색 클릭" },
  { id: "album", section: "Hitomi", target: '[data-tour="hitomi-albums"]', title: "앨범 상세", description: "한 번 클릭: 포커스 · Ctrl/Shift: 다중 선택", action: "dblclick", actionSelector: '[data-tour="hitomi-album"]:not([aria-disabled="true"])', hint: "작가·그룹이 있는 카드 더블클릭 또는 Enter" },
  { id: "detail", section: "Hitomi", target: '[data-tour="hitomi-detail-actions"]', title: "앨범 도구", description: "즐겨찾기 · 코멘트 · 다운로드" },
  { id: "follow", section: "Hitomi", target: '[data-tour="hitomi-detail-follow"]', spotlightSelector: '[data-tour-follow-kind]', title: "작가·그룹 즐겨찾기", description: "우클릭을 통해 즐겨찾기에 등록하세요\n즐겨찾기는 자동 탐색 대상에 포함됩니다.", action: "contextmenu", actionSelector: followTarget.split(", ").map(s => `${s}:not([data-favorite="true"])`).join(", "), satisfiedSelector: followTarget.split(", ").map(s => `${s}[data-favorite="true"]`).join(", "), waitForResult: true, hint: "키보드: Shift+F10", satisfiedHint: "이미 등록됨 · 다음으로 진행", emptyActionFallback: { step: "album", message: "이 앨범에는 작가·그룹 정보가 없습니다.", label: "다른 앨범 선택" } },
  { id: "auto-find", section: "Hitomi", target: '[data-tour="hitomi-nav-auto-find"]', title: "Auto Find", description: "즐겨찾기 작가·그룹의 앨범 탐색", action: "click", hint: "Auto Find 클릭" },
  { id: "auto-find-refresh", section: "Hitomi", target: '[data-tour="hitomi-auto-find-refresh"]', title: "자동 탐색 갱신", description: "갱신을 눌러야 탐색이 시작됩니다. 다운로드는 별도로 선택.", action: "click", waitForResult: true, satisfiedSelector: ':scope[data-running="true"]', hint: "즐겨찾기 작가·그룹 갱신 클릭", satisfiedHint: "탐색 진행 중 · 다음으로 진행" },
  { id: "downloads", section: "Hitomi", target: '[data-tour="hitomi-nav-downloads"]', title: "Downloads", description: "전체 · 작가 · 기간별 분류", action: "click", hint: "Downloads 클릭" },
  { id: "favorites", section: "Hitomi", target: '[data-tour="hitomi-favorites"]', title: "내 즐겨찾기", description: "앨범 · 페이지. 작가·그룹 즐겨찾기와는 별개.", action: "click", hint: "내 즐겨찾기 클릭" },
  { id: "activity", section: "공통", target: '[data-tour="hitomi-activity"]', title: "활동 기록", description: "대기 큐 · 작업 중 · 판본 검토", action: "click", hint: "활동 기록 클릭" },
  { id: "activity-info", section: "공통", target: '#activity-panel', title: "작업 상태", description: "탐색·다운로드는 화면을 바꿔도 계속 진행됩니다." },
  { id: "community", section: "공통", target: '[data-tour="community-nav"]', title: "커뮤니티", description: "작품 미리보기와 후기·별점. 내 후기는 따로 모아볼 수 있습니다.", action: "click", hint: "커뮤니티 클릭" },
  { id: "finish", section: "공통", target: '[data-tour="source-menu"]', title: "서비스 전환", description: "Danbooru·CHZZK 안내는 각 서비스 첫 방문 시 표시. 다시보기는 설정 → 일반." },
];

export const danbooruTourSteps: readonly TourStep[] = [
  { id: "danbooru-explore", section: "Danbooru", target: '[data-tour="danbooru-nav-explore"]', title: "Explore", description: "태그 · post 번호 검색. Hitomi와 조건·결과는 별도.", action: "click", hint: "Explore 클릭" },
  { id: "danbooru-search", section: "Danbooru", target: '[data-tour="danbooru-search"]', title: "검색 조건", description: "빈 검색은 최신 목록. 등급·파일 형식은 상세 조건에서 설정. 조건 수 제한은 검색창 아래에 표시.", action: "click", hint: "검색 클릭" },
  { id: "danbooru-post", section: "Danbooru", target: '[data-tour="danbooru-post"]', title: "post 상세", description: "원본 미리보기 · 작가 · 태그 · 관련 post", action: "click", hint: "미리보기 클릭" },
  { id: "danbooru-detail", section: "Danbooru", target: '[data-tour="danbooru-detail"]', title: "post 이동 · 저장", description: "A/D 또는 ←/→로 이동. 저장한 원본은 Downloads에서 확인." },
];

export const chzzkTourSteps: readonly TourStep[] = [
  { id: "chzzk-live", section: "CHZZK", target: '[data-tour="chzzk-nav-live"]', title: "라이브", description: "실시간 영상 · 채팅", action: "click", hint: "라이브 클릭" },
  { id: "chzzk-channels", section: "CHZZK", target: '[data-tour="chzzk-channels"]', title: "방송 보기", description: "채널 선택 · 주소 입력 · 로그인", action: "click", hint: "방송 보기 클릭" },
  { id: "chzzk-connection", section: "CHZZK", target: '[data-tour="chzzk-connection"]', title: "채널 연결", description: "주소 입력 시 채널 이름·프로필 표시. 별표로 즐겨찾기 등록." },
  { id: "chzzk-mado", section: "CHZZK", target: '[data-tour="chzzk-mado-mode"]', title: "마도모드", description: "최대 네 채널을 함께 시청.\n1화면4챗: 선택한 영상 하나 + 네 채널 채팅\n4화면4챗: 네 채널의 영상 + 채팅" },
  { id: "chzzk-auto", section: "CHZZK", target: '[data-tour="chzzk-nav-auto-record"]', title: "자동 녹화", description: "앱 실행 중 등록 채널의 방송 시작을 확인합니다.", action: "click", hint: "자동 녹화 클릭" },
  { id: "chzzk-auto-info", section: "CHZZK", target: '[data-tour="chzzk-auto-register"]', title: "채널 등록", description: "방송 중인 채널은 등록 즉시 녹화를 시도합니다. 로그인·권한 확인은 라이브에서. 안내 중에는 등록하지 않습니다." },
  { id: "chzzk-recordings", section: "CHZZK", target: '[data-tour="chzzk-nav-recordings"]', title: "녹화 목록", description: "저장본 재생 · 저장 상태 확인. 녹화 중에도 라이브 시청 가능.", action: "click", hint: "녹화 목록 클릭" },
];

export const tutorialStepsBySource: Record<ContentSource, readonly TourStep[]> = {
  hitomi: hitomiTourSteps,
  danbooru: danbooruTourSteps,
  chzzk: chzzkTourSteps,
};
