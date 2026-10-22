import { expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
import { invoke } from '@tauri-apps/api/core';
import { youtubeAutoRecordApi } from './youtubeAutoRecord';
it('uses only provider-local commands and an explicit expected revision', async () => {
  await youtubeAutoRecordApi.snapshot();
  await youtubeAutoRecordApi.resolve('https://www.youtube.com/@channel');
  await youtubeAutoRecordApi.set('UC4QobU6STFB0P71PMvOGN5A','채널',true,8);
  expect(invoke).toHaveBeenNthCalledWith(1,'youtube_auto_record_snapshot');
  expect(invoke).toHaveBeenNthCalledWith(2,'youtube_auto_record_resolve',{ input:'https://www.youtube.com/@channel' });
  expect(invoke).toHaveBeenNthCalledWith(3,'youtube_auto_record_set',{ channelId:'UC4QobU6STFB0P71PMvOGN5A',name:'채널',enabled:true,expectedRevision:8 });
});
