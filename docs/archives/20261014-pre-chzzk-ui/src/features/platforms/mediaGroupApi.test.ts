import { beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { platformMediaApi } from './platformMedia';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
beforeEach(() => vi.mocked(invoke).mockClear());
it('uses the exact bounded native group-summary command and opaque cursor', async () => {
  await platformMediaApi.groups!('youtube');
  expect(invoke).toHaveBeenLastCalledWith('platform_media_library_groups', { provider: 'youtube', cursor: null, limit: 100 });
  await platformMediaApi.groups!('soop', 'opaque-summary');
  expect(invoke).toHaveBeenLastCalledWith('platform_media_library_groups', { provider: 'soop', cursor: 'opaque-summary', limit: 100 });
});
it('keeps group lookup and member cursor separate from actual file commands', async () => {
  await platformMediaApi.groupMembers!('soop', 'opaque-group', 'opaque-members');
  expect(invoke).toHaveBeenLastCalledWith('platform_media_library_group_members', { provider: 'soop', groupKey: 'opaque-group', cursor: 'opaque-members', limit: 100 });
  await platformMediaApi.delete('actual-library-id', 9);
  expect(invoke).toHaveBeenLastCalledWith('platform_media_delete', { libraryId: 'actual-library-id', revision: 9 });
});
