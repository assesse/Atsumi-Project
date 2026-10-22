import { invoke } from '@tauri-apps/api/core';
import type { NativeViewport } from '../features/platforms/nativePlayerGeometry';
import type { PlatformScreenshotItem } from '../features/platforms/platformScreenshots';
export type SoopScreenshotSnapshot = { item: PlatformScreenshotItem; preview: string };
export type SoopOriginalSnapshot = { token:string; channelId:string; broadcastId:string; state:string; authenticated:boolean|null; chatVisible:boolean; captureReady:boolean; recordingJob:string|null; recordingStopping:boolean; fullscreen:boolean; error:string|null };
export type SoopAccountSnapshot = { authenticated:boolean|null; open:boolean; error:string|null };
export const soopOriginalApi = {
  open: (channelId:string,requestId:string)=>invoke<SoopOriginalSnapshot>('soop_original_open',{channelId,requestId}),
  snapshot: (token:string)=>invoke<SoopOriginalSnapshot>('soop_original_snapshot',{token}),
  viewport: (token:string,viewport:NativeViewport)=>invoke<void>('soop_original_viewport',{token,viewport}),
  close: (token:string)=>invoke<void>('soop_original_close',{token}),
  controls: (token:string,canClose:boolean,captureBusy:boolean)=>invoke<void>('soop_original_controls',{token,canClose,captureBusy}),
  resume: (token:string)=>invoke<void>('soop_original_resume',{token}),
  record: (token:string,enabled:boolean)=>invoke<SoopOriginalSnapshot>('soop_original_record',{token,enabled}),
  login: ()=>invoke<SoopAccountSnapshot>('soop_original_login'),
  logout: ()=>invoke<SoopAccountSnapshot>('soop_original_logout'),
  account: ()=>invoke<SoopAccountSnapshot>('soop_original_account'),
  screenshot: (token:string,requestId:string)=>invoke<SoopScreenshotSnapshot>('soop_original_screenshot',{token,requestId}),
  undoScreenshot: (token:string,id:string)=>invoke<void>('soop_original_screenshot_undo',{token,id}),
  fullscreen: (token:string,enabled:boolean)=>invoke<void>('soop_original_fullscreen',{token,enabled}),
};
export type SoopOriginalApi = Omit<typeof soopOriginalApi,'controls'|'resume'> & Partial<Pick<typeof soopOriginalApi,'controls'|'resume'>>;
