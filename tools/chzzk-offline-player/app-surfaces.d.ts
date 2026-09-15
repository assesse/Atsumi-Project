import type { ComponentType, ReactNode, Ref } from 'react';
export function createSurfaces(react: unknown, reactDOM: unknown, textarea: unknown, fixture: Record<string, unknown>, notify: (message: string) => void): {
  BroadcastInfo: ComponentType<{ wide?: boolean; replay?: boolean; title?: string; channelName?: string; recordedAt?: number | null; profileImage?: string }>;
  ReplayShell: ComponentType<{ children?: ReactNode; listRef?: Ref<HTMLDivElement>; onScroll?: () => void; headerControls?: ReactNode; onCollapse?: (() => void) | null; footer?: ReactNode; floatingContent?: ReactNode; title?: string }>;
  ReplaySearch: ComponentType<{ query: string; count: number; countText?: string; onQuery: (query: string) => void }>;
};
