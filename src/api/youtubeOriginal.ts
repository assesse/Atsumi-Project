import { invoke } from '@tauri-apps/api/core';
import type { NativeViewport } from '../features/platforms/nativePlayerGeometry';
import type { PlatformScreenshotItem } from '../features/platforms/platformScreenshots';
export type YouTubeOriginalSnapshot = { token:string; channelId:string; broadcastId:string; canonicalChannelId?:string|null; state:string; authenticated:boolean|null; playability?:'LOGIN_REQUIRED'|'UNAVAILABLE'; chatVisible:boolean; captureReady:boolean; captureUnavailableReason?:string|null; recordingJob:string|null; recordingStopping:boolean; fullscreen:boolean; error:string|null };
export type YouTubeAccountSnapshot = { authenticated:boolean|null; open:boolean; error:string|null };
export type YouTubeVodClock = {token:string;videoId:string;documentEpoch:number;sourceId:string;sourceGeneration:number;sequence:number;seekGeneration:number;currentTime:number|null;duration:number|null;paused:boolean;seeking:boolean;playbackRate:number;buffering:boolean;ended:boolean;adState:'content'|'ad'|'unknown';valid:boolean;ageMs:number};
export type YouTubeVodControlRequest = Pick<YouTubeVodClock,'documentEpoch'|'sourceId'|'sourceGeneration'> & {requestId:string;action:'seek'|'pause'|'resume'|'rate';seconds?:number;rate?:number};
export const youtubeOriginalApi = {
  open: (channelId:string,requestId:string,playbackKind?:'live'|'vod')=>invoke<YouTubeOriginalSnapshot>('youtube_original_open',{channelId,requestId,...(playbackKind?{playbackKind}:{})}),
  snapshot: (token:string)=>invoke<YouTubeOriginalSnapshot>('youtube_original_snapshot',{token}),
  vodClock: (token:string)=>invoke<YouTubeVodClock|null>('youtube_original_vod_clock',{token}),
  vodControl: (token:string,request:YouTubeVodControlRequest)=>invoke<void>('youtube_original_vod_control',{token,request}),
  viewport: (token:string,viewport:NativeViewport)=>invoke<void>('youtube_original_viewport',{token,viewport}),
  close: (token:string)=>invoke<void>('youtube_original_close',{token}),
  controls: (token:string,canClose:boolean,captureBusy:boolean)=>invoke<void>('youtube_original_controls',{token,canClose,captureBusy}),
  resume: (token:string)=>invoke<void>('youtube_original_resume',{token}),
  record: (token:string,enabled:boolean)=>invoke<YouTubeOriginalSnapshot>('youtube_original_record',{token,enabled}),
  login: ()=>invoke<YouTubeAccountSnapshot>('youtube_original_login'),
  logout: ()=>invoke<YouTubeAccountSnapshot>('youtube_original_logout'),
  account: ()=>invoke<YouTubeAccountSnapshot>('youtube_original_account'),
  fullscreen: (token:string,enabled:boolean)=>invoke<void>('youtube_original_fullscreen',{token,enabled}),
  screenshot: (token:string,requestId:string)=>invoke<PlatformScreenshotItem>('youtube_original_screenshot',{token,requestId}),
  undoScreenshot: (token:string,id:string)=>invoke<void>('youtube_original_screenshot_undo',{token,id}),
};
/** An owned replay uses its public session token for every event. Supplying this
 * source replaces ordinary shared-viewer events, including same-video events. */
export type YouTubeOriginalEvents = {
  action(listener: (value: { token: string; action: string }) => void): Promise<() => void>;
  clock(listener: (value: YouTubeVodClock) => void): Promise<() => void>;
  fullscreen(listener: (value: { token: string; enabled: boolean }) => void): Promise<() => void>;
  screenshot(listener: (value: { token: string }) => void): Promise<() => void>;
};
export type YouTubeOriginalApi = Omit<typeof youtubeOriginalApi,'controls'|'resume'|'vodClock'|'vodControl'> & Partial<Pick<typeof youtubeOriginalApi,'controls'|'resume'|'vodClock'|'vodControl'>> & { events?: YouTubeOriginalEvents; cancelOpen?(requestId:string):Promise<void> };
