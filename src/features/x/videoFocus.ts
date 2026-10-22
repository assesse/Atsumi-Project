import {useEffect,useRef,useState,type RefObject} from "react";

// Only one app-owned player nearest a visible pane's center. Preview priority
// is supplied by the workspace, never by an offscreen site reader.
type Candidate={element:HTMLElement;notify:(chosen:boolean)=>void;revoke:()=>void;priority:number};
const candidates=new Set<Candidate>();
let listening=false,frame=0;

function update(){
  frame=0;
  let best:Candidate|undefined,distance=Infinity;
  if(document.visibilityState!=='hidden')for(const item of candidates){
    const fullscreen=document.fullscreenElement;
    const ownsFullscreen=!!fullscreen&&(fullscreen===item.element||item.element.contains(fullscreen));
    if(fullscreen&&!ownsFullscreen&&!fullscreen.contains(item.element))continue;
    const rect=item.element.getBoundingClientRect();
    const pane=ownsFullscreen?undefined:item.element.closest('.x-scroll,.x-preview-scroll')?.getBoundingClientRect();
    const top=Math.max(0,pane?.top??0),bottom=Math.min(innerHeight,pane?.bottom??innerHeight);
    const left=Math.max(0,pane?.left??0),right=Math.min(innerWidth,pane?.right??innerWidth);
    const visible=Math.min(bottom,rect.bottom)-Math.max(top,rect.top);
    if(rect.height<=0||rect.width<=0||bottom<=top||Math.min(right,rect.right)<=Math.max(left,rect.left)||visible<Math.min(rect.height*.5,180))continue;
    const delta=Math.abs((rect.top+rect.bottom)/2-(top+bottom)/2);
    if(item.priority<10&&delta>Math.max(140,(bottom-top)*.3))continue;
    if(!best||item.priority>best.priority||item.priority===best.priority&&delta<distance){best=item;distance=delta;}
  }
  // Revoke old native playback before React can start its replacement. Hidden
  // documents may not receive any animation frame, so revocation is synchronous.
  for(const item of candidates)if(item!==best)item.revoke();
  for(const item of candidates)item.notify(item===best);
}
function schedule(){if(!frame)frame=requestAnimationFrame(update);}
function visibilityChanged(){if(frame){cancelAnimationFrame(frame);frame=0;}update();}

export function useCenteredVideo(ref:RefObject<HTMLElement|null>,enabled:boolean,priority=0,onRevoked?:()=>void){
  const [chosen,setChosen]=useState(false),revoke=useRef(onRevoked);
  revoke.current=onRevoked;
  useEffect(()=>{
    if(!enabled||!ref.current){setChosen(false);revoke.current?.();return;}
    const item={element:ref.current,notify:setChosen,revoke:()=>revoke.current?.(),priority};
    candidates.add(item);
    if(!listening){
      listening=true;
      window.addEventListener('scroll',schedule,true);window.addEventListener('resize',schedule);window.addEventListener('load',schedule,true);
      document.addEventListener('visibilitychange',visibilityChanged);document.addEventListener('fullscreenchange',schedule);
    }
    const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(schedule);
    observer?.observe(item.element);
    const pane=item.element.closest('.x-scroll,.x-preview-scroll');if(pane)observer?.observe(pane);
    const intersection=typeof IntersectionObserver==='undefined'?null:new IntersectionObserver(schedule,{threshold:[0,.25,.5,.75,1]});
    intersection?.observe(item.element);schedule();
    return()=>{
      item.revoke();candidates.delete(item);observer?.disconnect();intersection?.disconnect();setChosen(false);
      if(candidates.size)schedule();
      else{
        if(frame){cancelAnimationFrame(frame);frame=0;}
        window.removeEventListener('scroll',schedule,true);window.removeEventListener('resize',schedule);window.removeEventListener('load',schedule,true);
        document.removeEventListener('visibilitychange',visibilityChanged);document.removeEventListener('fullscreenchange',schedule);listening=false;
      }
    };
  },[ref,enabled,priority]);
  return enabled&&chosen;
}
