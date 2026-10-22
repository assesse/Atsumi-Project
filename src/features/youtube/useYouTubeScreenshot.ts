import { useEffect, useRef, useState } from 'react';
import { platformScreenshotsApi, type PlatformScreenshotsApi } from '../platforms/platformScreenshots';
import type { YouTubeOriginalApi } from '../../api/youtubeOriginal';
type Preview={id:string;url:string;undoing:boolean;error:string;undoDeadline:number};
/** Native host captures only the existing official video; no second player. */
export function useYouTubeScreenshot(token:string|null,enabled:boolean,api:YouTubeOriginalApi,images:Pick<PlatformScreenshotsApi,'image'>=platformScreenshotsApi){
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[preview,setPreview]=useState<Preview|null>(null);
  const epoch=useRef(0),flight=useRef(false),previewRef=useRef<Preview|null>(null);
  const dismiss=(expectedId?:string)=>{if(expectedId&&previewRef.current?.id!==expectedId)return;previewRef.current=null;setPreview(null);};
  useEffect(()=>{++epoch.current;flight.current=false;setBusy(false);setError('');dismiss();return()=>{++epoch.current;previewRef.current=null;};},[token,enabled,api,images]);
  const save=async()=>{if(!enabled||!token||flight.current||previewRef.current?.undoing)return;const current=epoch.current;flight.current=true;setBusy(true);setError('');dismiss();
    const undoDeadline=performance.now()+45_000;
    try{const item=await api.screenshot(token,crypto.randomUUID());if(current!==epoch.current)return;const url=await images.image('youtube',item.id,true);if(current!==epoch.current)return;
      if(!/^data:image\/(?:png|jpeg|webp);base64,[A-Za-z0-9+/=]+$/.test(url)||url.length>24*1024*1024)throw new Error('미리보기를 불러오지 못했습니다. 스크린샷 목록에서 확인해 주세요.');
      const shown={id:item.id,url,undoing:false,error:'',undoDeadline};previewRef.current=shown;setPreview(shown);
    }catch(cause){if(current===epoch.current)setError(typeof cause==='string'?cause:cause instanceof Error?cause.message:'스크린샷 저장에 실패했습니다.');}
    finally{if(current===epoch.current){flight.current=false;setBusy(false);}}
  };
  const undo=async()=>{const shown=previewRef.current;if(!enabled||!token||!shown||shown.undoing||shown.error||performance.now()>=shown.undoDeadline)return;const current=epoch.current;const next={...shown,undoing:true};previewRef.current=next;setPreview(next);
    try{await api.undoScreenshot(token,shown.id);if(current===epoch.current&&previewRef.current===next)dismiss(shown.id);}catch{if(current===epoch.current&&previewRef.current===next){const failed={...shown,undoing:false,error:'취소를 확인하지 못했습니다. 스크린샷 목록에서 확인해 주세요.'};previewRef.current=failed;setPreview(failed);setError(failed.error);}}
  };
  return{busy:enabled&&busy,error:enabled?error:'',preview:enabled?preview:null,save,undo,dismiss,clearError:()=>{setError('');if(previewRef.current?.error)dismiss();}};
}
