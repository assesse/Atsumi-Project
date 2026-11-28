import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlatformReplayChat, type PlatformReplayChatProps, type ReplayChatApi, type ReplayChatItem, type ReplayChatPage } from '../platforms/PlatformReplayChat';
import type { SoopMissionMetadata } from '../platforms/platformMedia';
import { validSoopMission } from './soopMission';

const id='a'.repeat(64);
const image='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aRZkAAAAASUVORK5CYII=';
const gift:SoopMissionMetadata={kind:'battle',event:'gift',count:23,relay:true,imageAssetId:id};
const row=(m:SoopMissionMetadata):ReplayChatItem=>({id:`${m.kind}-${m.event}`,timeMs:1000,kind:m.event==='gift'?'donation':'system',author:m.event==='gift'?'Saved viewer':'',text:'legacy public text',amount:null,badges:[],color:null,soopMission:m});
const page=(items:ReplayChatItem[]):ReplayChatPage=>({items,state:'ready',timelineQuality:'receive-time-approximate',nextAtMs:null,cursor:null,previousCursor:null,total:items.length,malformedLines:0,unsupportedItems:0,reordered:0,duplicates:0,partialFile:false});
function apiFor(items:ReplayChatItem[]){return {page:vi.fn().mockResolvedValue(page(items)),asset:vi.fn().mockResolvedValue({state:'ready',dataUrl:image})};}
let host:HTMLDivElement,root:Root;
async function render(api:ReplayChatApi,overrides:Partial<PlatformReplayChatProps>={}){await act(async()=>root.render(<PlatformReplayChat provider="soop" libraryId="recording" currentTime={2} active privacyMode={false} api={api} {...overrides}/>));}
function content():HTMLElement{return host.querySelector('aside')?.shadowRoot?.querySelector<HTMLElement>('.soop-original-chat-root')??host;}

describe('SOOP service121 saved original presentation',()=>{
  beforeEach(()=>{Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});host=document.createElement('div');document.body.append(host);root=createRoot(host);});
  afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();});

  it.each(['battle','challenge'] as const)('keeps %s gifts and settlements semantically different',async kind=>{
    const api=apiFor([row({...gift,kind}),row({kind,event:'settle',count:0,imageAssetId:id})]);await render(api);
    const rows=content().querySelectorAll('[data-chat-row-id]');expect(rows).toHaveLength(2);
    expect(rows[0]!.querySelector('.donation-container.basic .info-box .name')).toHaveTextContent('Saved viewer');
    expect(rows[0]!.querySelector('.money')).toHaveTextContent(`${kind==='battle'?'대결':'도전'}미션 별풍선23개 후원`);
    expect(rows[0]!.querySelector('.money em')).toHaveTextContent('23');
    expect(rows[1]!.querySelector('.donation-container')).not.toHaveClass('basic');
    expect(rows[1]!.querySelector('.name')).toBeNull();expect(rows[1]!.querySelector('.money')).toHaveTextContent('별풍선 0개 획득!');
    expect(content().querySelector('.count-box,.add-img,[user_id],[user_nick],img[src^="http"]')).toBeNull();
    expect(api.asset).toHaveBeenCalledTimes(1);expect(api.asset).toHaveBeenCalledWith('recording',id);
    expect(content().querySelectorAll('.ceremony-img.custom-mobile-img img')).toHaveLength(2);
  });
  it.each([
    [{kind:'battle',event:'notice',notice:{result:'draw'}},'결과 무승부입니다!'],
    [{kind:'battle',event:'notice',notice:{result:'winner',winner:'Saved team'}},'승리 팀은 Saved team입니다.'],
    [{kind:'battle',event:'notice',notice:{result:'rank',rank:2,team:'Saved team'}},'2등은 Saved team입니다.'],
    [{kind:'challenge',event:'notice',notice:{result:'challenge',title:'Saved title',successful:true}},'[Saved title] 미션을 성공하였습니다!'],
    [{kind:'challenge',event:'notice',notice:{result:'challenge',title:'Saved title',successful:false}},'[Saved title] 미션을 실패하였습니다...'],
  ] satisfies Array<[SoopMissionMetadata,string]>)('uses original notice bubble without donor or donation for %j',async(meta,text)=>{
    const api=apiFor([row(meta)]);await render(api);
    expect(content().querySelector('.bubble-container')).not.toHaveClass('notice');
    expect(content().querySelector('.bubble-container > .title > .ic-notice')).not.toBeNull();
    expect(content().querySelector('.bubble-container > .msg')).toHaveTextContent(text);
    expect(content().querySelector('.donation-container,.name,.ceremony-img')).toBeNull();expect(api.asset).not.toHaveBeenCalled();
  });
  it('retains text when captured artwork is unavailable or cannot decode, without fetching a current substitute',async()=>{
    const api=apiFor([row(gift)]);await render(api);expect(content().querySelector('.ceremony-img img')).toHaveAttribute('src',image);
    await act(async()=>content().querySelector('.ceremony-img img')!.dispatchEvent(new Event('error')));
    expect(content().querySelector('.ceremony-img img')).toBeNull();expect(content().querySelector('.info-box small')).toHaveTextContent('이미지 없음');
    expect(content().querySelector('.money')).toHaveTextContent('23개 후원!');expect(api.asset).toHaveBeenCalledTimes(1);
    api.page.mockResolvedValue(page([row({...gift,imageAssetId:'https://injected/image.png'})]));await render(api,{libraryId:'other'});
    expect(content().querySelector('.ceremony-img img')).toBeNull();expect(api.asset).toHaveBeenCalledTimes(1);
  });
  it('preserves public fan announcements and saved-profile actions without a payment or HTML action',async()=>{
    const r={...row({...gift,fanOrder:3,topFan:true}),author:'<img src=x>'};await render(apiFor([r]));
    expect(content().querySelector('[user-type="fan"]')).toHaveTextContent('<img src=x>님이 3번째 팬클럽이 되셨습니다.');
    expect(content().querySelector('[user-type="vip"]')).toHaveTextContent('열혈팬이 되셨습니다.');
    expect(content().querySelector('.info-box .name img,[user_id],a[href]')).toBeNull();
    await act(async()=>(content().querySelector('.info-box .name')as HTMLButtonElement).click());
    expect(content().querySelector('[aria-label="채팅 사용자 정보"]')).toHaveTextContent('저장 당시 정보');
    expect(content().querySelectorAll('.actionbox button:not(:disabled)')).toHaveLength(0);
  });
  it('rejects inconsistent events and keeps platform/privacy boundaries',async()=>{
    const api=apiFor([{...row(gift),kind:'chat'}]);await render(api);expect(api.asset).not.toHaveBeenCalled();expect(content().querySelector('.donation-container')).toBeNull();
    api.page.mockResolvedValue(page([row(gift)]));await render(api,{provider:'youtube',libraryId:'youtube'});
    expect(api.asset).not.toHaveBeenCalled();expect(content().querySelector('.donation-container')).toBeNull();
    await render(api,{privacyMode:true,libraryId:'hidden'});expect(content().querySelector('.donation-container,img')).toBeNull();
    for(const bad of [{...gift,count:-1},{...gift,count:0},{...gift,event:'settle'},{...gift,kind:'challenge',fanOrder:2},
      {kind:'battle',event:'notice',notice:{result:'challenge',title:'x',successful:true}}])expect(validSoopMission(bad)).toBe(false);
  });
});
