import {act,StrictMode} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {XWorkspace} from "./XCleanWorkspace";
import {type XApi,type XPage,type XPost} from "./api";
const post:XPost={id:"123456789",handle:"artist",name:"Artist",text:"풍경 사진",time:"2026-10-18T00:00:00Z",media:[],video:false,truncated:false,stats:{replies:"2",reposts:"3",likes:"4",views:"100",bookmarks:"1",liked:false,reposted:false,bookmarked:false,canReply:true,canRepost:true,canLike:true,canBookmark:true}};
const page=(posts=[post]):XPage=>({posts,cursor:"cursor",generation:1,status:"signedIn",limited:false,communities:[{id:"987654321",name:"작업 모임",path:"/i/communities/987654321"}]});
const mock=():XApi=>({load:vi.fn().mockResolvedValue(page()),cancel:vi.fn().mockResolvedValue(undefined),suspend:vi.fn().mockResolvedValue(undefined),account:vi.fn(),select:vi.fn().mockResolvedValue({generation:2,status:"signedIn",open:false,active:"b",accounts:[{id:"b",handle:"bob"}]}),status:vi.fn().mockResolvedValue({generation:1,status:"signedIn",open:false,active:"a",accounts:[{id:"a",handle:"alice"},{id:"b",handle:"bob"}]}),image:vi.fn().mockResolvedValue("data:image/jpeg;base64,YQ=="),open:vi.fn().mockResolvedValue(undefined)});
let root:Root,host:HTMLDivElement;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
const render=(api:XApi,active=true,privacyMode=false)=>act(async()=>root.render(<XWorkspace api={api} active={active} privacyMode={privacyMode} runtime="browser-mock" onNotice={vi.fn()}/>));
const button=(name:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent===name||b.getAttribute("aria-label")===name)!;
const click=(name:string)=>act(async()=>button(name).click());
it("renders own articles and one account-specific header, not an official web slot",async()=>{
  await render(mock());expect(host.querySelectorAll(".x-toolbar")).toHaveLength(1);expect(host.querySelector("article")).toHaveTextContent("풍경 사진");
  expect(host.querySelector("iframe,.x-native-feed")).toBeNull();expect(host.textContent).not.toContain("가입된 X 커뮤니티");expect(host.textContent).not.toContain("무슨 일이 일어나");
  expect([...host.querySelectorAll(".x-feeds button")].map(b=>b.textContent)).toEqual(["추천","팔로잉","작업 모임"]);
  expect(host.querySelectorAll(".x-collections button svg")).toHaveLength(2);
});
it("names current account recovery without changing its existing-profile operation",async()=>{
  const api=mock();vi.mocked(api.account).mockResolvedValue({generation:2,status:"signedIn",open:true,active:"a",accounts:[{id:"a",handle:"alice"}]});
  await render(api);await click("계정");
  const labels=[...host.querySelectorAll('[role="menuitem"]')].map(b=>b.textContent);
  expect(labels).toEqual(["@alice ✓","@bob","계정 추가","현재 계정 정보 복구"]);expect(labels).not.toContain("다시 로그인");
  await click("현재 계정 정보 복구");expect(api.account).toHaveBeenCalledExactlyOnceWith(false);expect(api.select).not.toHaveBeenCalled();
});
it("keeps account addition separate from current account recovery",async()=>{
  const api=mock();vi.mocked(api.account).mockResolvedValue({generation:2,status:"signedOut",open:true,accounts:[]});
  await render(api);await click("계정");await click("계정 추가");expect(api.account).toHaveBeenCalledExactlyOnceWith(true);
});
it("does not expose current account recovery while privacy mode is enabled",async()=>{
  const api=mock();await render(api);await click("계정");expect(button("현재 계정 정보 복구")).toBeDefined();
  await render(api,true,true);expect(host.querySelector('[role="menu"]')).toBeNull();expect(api.account).not.toHaveBeenCalled();
});
it("routes actual communities and own collections through bounded reader keys",async()=>{
  const api=mock();await render(api);await click("작업 모임");expect(api.load).toHaveBeenLastCalledWith("community:987654321",null,expect.any(String));
  await click("마음함");expect(api.load).toHaveBeenLastCalledWith("likes",null,expect.any(String));
  await click("북마크");expect(api.load).toHaveBeenLastCalledWith("bookmarks",null,expect.any(String));
});
it("retains already loaded tabs and scroll instead of resetting the entire feed",async()=>{
  const api=mock();await render(api);await click("팔로잉");await click("추천");expect(api.load).toHaveBeenCalledTimes(2);
});
it("appends pages by ID without duplicates",async()=>{
  const api=mock();await render(api);vi.mocked(api.load).mockResolvedValueOnce(page([post,{...post,id:"555555555",text:"새 게시물"}]));await click("더 보기");expect(host.querySelectorAll(".x-timeline>article")).toHaveLength(2);
});
it("shows the selected post and actual thread replies inside the app",async()=>{
  const api=mock();await render(api);vi.mocked(api.load).mockResolvedValueOnce(page([post,{...post,id:"555555555",text:"답글 내용"}]));await click("Artist 게시물 미리보기");
  expect(api.load).toHaveBeenLastCalledWith("post:123456789",null,expect.any(String));expect(host.querySelector(".x-thread")).toHaveTextContent("답글 내용");expect(host.querySelector(".x-thread .x-actions")).not.toBeNull();
});
it("shows a skeleton while the first page is pending",async()=>{
  const api=mock();vi.mocked(api.load).mockReturnValue(new Promise(()=>{}));await render(api);expect(host.querySelector('[aria-label="피드 불러오는 중"]')).not.toBeNull();
});
it('opens an image viewer without changing the selected post, then restores focus',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,media:['https://pbs.twimg.com/media/A.jpg','https://pbs.twimg.com/media/B.jpg']}]));await render(api);
  const source=button('이미지 1 크게 보기');source.focus();await click('이미지 1 크게 보기');
  expect(host.querySelector('[role="dialog"]')).toHaveAttribute('aria-label','이미지 크게 보기');expect(api.load).toHaveBeenCalledTimes(1);
  await click('다음 이미지');expect(host.querySelector('.x-image-viewer header')).toHaveTextContent('2 / 2');
  await click('이미지 닫기');expect(host.querySelector('[role="dialog"]')).toBeNull();expect(document.activeElement).toBe(source);
});
it('opens the author inside the app and restores the prior feed',async()=>{
  const api=mock();await render(api);await click('Artist 프로필 보기');expect(api.load).toHaveBeenLastCalledWith('profile:artist',null,expect.any(String));
  expect(host.querySelector('.x-profile-heading')).toHaveTextContent('@artist');await click('이전 피드로');expect(button('추천')).toHaveAttribute('aria-pressed','true');expect(api.load).toHaveBeenCalledTimes(2);
});
it('separates replies, related posts and unclassified results without inventing relations',async()=>{
  const api=mock();await render(api);vi.mocked(api.load).mockResolvedValueOnce(page([post,{...post,id:'555555555',relation:'reply',text:'대화'},{...post,id:'666666666',relation:'related',text:'추천된 글'},{...post,id:'777777777',text:'미확인'}]));await click('Artist 게시물 미리보기');
  expect(host.querySelector('section[aria-label="답글"]')).toHaveTextContent('대화');expect(host.querySelector('section[aria-label="관련 게시물"]')).toHaveTextContent('추천된 글');expect(host.querySelector('section[aria-label="분류 미확인 게시물"]')).toHaveTextContent('미확인');
});
it('opens repost context as a read-only post, never submitting a repost',async()=>{
  const api=mock();api.action=vi.fn();vi.mocked(api.load).mockResolvedValue(page([{...post,repost:'친구님이 재게시'}]));await render(api);await click('재게시된 게시물 보기');expect(api.load).toHaveBeenLastCalledWith('post:123456789',null,expect.any(String));expect(api.action).not.toHaveBeenCalled();
});
it('shows a pinned label without misrepresenting it as an interactive repost',async()=>{
  const api=mock();api.action=vi.fn();vi.mocked(api.load).mockResolvedValue(page([{...post,pinned:true,repost:null}]));await render(api);
  expect(host.querySelector('.x-post')).toHaveTextContent('고정됨');expect(host.querySelector('[aria-label="재게시된 게시물 보기"]')).toBeNull();
  expect(api.action).not.toHaveBeenCalled();expect(api.load).toHaveBeenCalledTimes(1);
});
it("does not turn failed collections into empty success",async()=>{
  const api=mock();vi.mocked(api.load).mockRejectedValue(new Error("목록 구성 확인 실패"));await render(api);expect(host.querySelector('[role="alert"]')).toHaveTextContent("목록 구성 확인 실패");expect(host.textContent).not.toContain("아직 게시물이 없습니다");
});
it("never starts requests while private or inactive",async()=>{
  const api=mock();await render(api,false);await render(api,true,true);expect(api.load).not.toHaveBeenCalled();expect(host.querySelector("article")).toBeNull();expect(host.textContent).not.toContain("@alice");
});
it("ignores late feed data after privacy or account invalidation",async()=>{
  const api=mock();let done!:(p:XPage)=>void;vi.mocked(api.load).mockReturnValue(new Promise(r=>done=r));await render(api);await render(api,true,true);await act(async()=>done(page()));expect(host.querySelector("article")).toBeNull();expect(api.cancel).toHaveBeenCalled();
});
it("does not drop a later tab selection when an old feed finishes",async()=>{
  const api=mock();let done!:(p:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(r=>done=r));await render(api);await click("팔로잉");await act(async()=>done(page()));expect(button("팔로잉")).toHaveAttribute("aria-pressed","true");
});
it("only exposes external opening in the context menu",async()=>{
  const api=mock();await render(api);expect(host.textContent).not.toContain("원문");expect(host.textContent).not.toContain("↗");
  await act(async()=>host.querySelector("article")!.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true,clientX:200,clientY:200})));
  await click("외부 브라우저에서 열기");expect(api.open).toHaveBeenCalledWith(post.id);
});
it("does not optimistically claim social changes and prevents duplicate clicks",async()=>{
  const api=mock();let done!:(p:XPost)=>void;api.action=vi.fn().mockReturnValue(new Promise(r=>done=r));await render(api);
  await click("마음에 들어요");await click("마음에 들어요");expect(api.action).toHaveBeenCalledTimes(1);expect(button("마음에 들어요")).toHaveAttribute("aria-pressed","false");
  await act(async()=>done({...post,stats:{...post.stats!,liked:true,likes:"5"}}));expect(button("마음에 들어요")).toHaveAttribute("aria-pressed","true");
});
it("only reads replies and reposts, without a composer or repost mutation",async()=>{
  const api=mock();api.action=vi.fn();vi.mocked(api.load).mockResolvedValue(page([{...post,repost:"친구님이 재게시함"}]));await render(api);
  expect(host.querySelector('.x-repost-context')).toHaveTextContent("친구님이 재게시함");
  expect(host.querySelector('button[aria-label="재게시"]')).toBeNull();expect(host.querySelector('.x-repost-count')).toHaveTextContent("3");
  await click("답글 보기");expect(api.load).toHaveBeenLastCalledWith("post:123456789",null,expect.any(String));
  expect(host.querySelector("textarea,form")).toBeNull();expect(host.textContent).not.toContain("답글 게시");expect(api.action).not.toHaveBeenCalled();
});
it("sends only the selected bookmark transition and can restore it",async()=>{
  const api=mock();api.action=vi.fn().mockResolvedValueOnce({...post,stats:{...post.stats!,bookmarked:true}}).mockResolvedValueOnce(post);await render(api);
  await click("게시물 북마크");expect(api.action).toHaveBeenLastCalledWith(post.id,"bookmark",1,false);expect(button("게시물 북마크")).toHaveAttribute("aria-pressed","true");
  await click("게시물 북마크");expect(api.action).toHaveBeenLastCalledWith(post.id,"bookmark",1,true);expect(button("게시물 북마크")).toHaveAttribute("aria-pressed","false");
});
// Scoped warning regressions use synthetic responses only; no account mutation occurs.
const pendingConfirmation="X의 처리 결과를 아직 확인하지 못했습니다. 중복 전송하지 말고 목록을 새로고침해 확인해 주세요.";
it.each([["like",true],["like",false],["bookmark",true],["bookmark",false]] as const)(
  "resolves only pending confirmation after a later exact fresh %s state read (%s)",async(kind,value)=>{
    const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
    const label=kind==="like"?"마음에 들어요":"게시물 북마크";
    await click(label);expect(host.textContent).toContain(pendingConfirmation);
    vi.mocked(api.load).mockResolvedValueOnce(page([{...post,handle:"ARTIST",stats:{
      ...post.stats!,liked:kind==="like"?value:false,bookmarked:kind==="bookmark"?value:false,
    }}]));
    await click("새로고침");
    expect(host.textContent).not.toContain(pendingConfirmation);expect(button(label)).toHaveAttribute("aria-pressed",String(value));
    expect(api.action).toHaveBeenCalledTimes(1);
  },
);
it("does not classify a general action error as resolved uncertainty",async()=>{
  const api=mock(),message="게시물 상태가 바뀌었습니다. 다시 불러온 뒤 확인해 주세요.";
  api.action=vi.fn().mockRejectedValue(new Error(message));await render(api);await click("마음에 들어요");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats:{...post.stats!,liked:true}}]));await click("새로고침");
  expect(host.textContent).toContain(message);expect(api.action).toHaveBeenCalledTimes(1);
});
it.each([
  ["no target",page([])],
  ["another post",page([{...post,id:"555555555"}])],
  ["another author",page([{...post,handle:"other"}])],
  ["another generation",{...page(),generation:2}],
  ["unknown session",{...page(),status:"unknown" as const}],
  ["missing stats",page([{...post,stats:undefined}])],
  ["unavailable like control",page([{...post,stats:{...post.stats!,canLike:false}}])],
  ["non-boolean like state",page([{...post,stats:{...post.stats!,liked:undefined as unknown as boolean}}])],
] as const)("retains pending confirmation when fresh evidence has %s",async(_label,response)=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  vi.mocked(api.load).mockResolvedValueOnce(response);await click("새로고침");
  expect(host.textContent).toContain(pendingConfirmation);expect(api.action).toHaveBeenCalledTimes(1);
});
it.each([
  {...post.stats!,canBookmark:false},
  {...post.stats!,bookmarked:undefined as unknown as boolean},
])("requires a real bookmark control and boolean for bookmark uncertainty",async stats=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("게시물 북마크");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats}]));await click("새로고침");
  expect(host.textContent).toContain(pendingConfirmation);
});
it("does not use a retained cached target as fresh append evidence",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,id:"555555555",text:"다른 새 행"}]));await click("더 보기");
  expect(host.querySelectorAll(".x-timeline>article")).toHaveLength(2);expect(host.textContent).toContain(pendingConfirmation);
});
it("does not clear uncertainty using a read already in flight before the warning",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click("새로고침");
  await click("마음에 들어요");expect(host.textContent).toContain(pendingConfirmation);
  await act(async()=>done(page([{...post,stats:{...post.stats!,liked:true}}])));
  expect(host.textContent).toContain(pendingConfirmation);
});
it("does not clear a newer warning with a response admitted under an older warning token",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click("새로고침");
  await click("마음에 들어요");await act(async()=>done(page([{...post,stats:{...post.stats!,liked:true}}])));
  expect(host.textContent).toContain(pendingConfirmation);expect(api.action).toHaveBeenCalledTimes(2);
});
it("does not clear another post's uncertainty when an unrelated action starts or succeeds",async()=>{
  const api=mock(),other={...post,id:"555555555",name:"Other"};
  vi.mocked(api.load).mockResolvedValue(page([post,other]));
  let done!:(value:XPost)=>void;api.action=vi.fn().mockRejectedValueOnce(new Error(pendingConfirmation)).mockReturnValueOnce(new Promise(resolve=>done=resolve));
  await render(api);await click("마음에 들어요");
  const otherAction=host.querySelector<HTMLButtonElement>('.x-timeline>article:nth-child(2) [aria-label="마음에 들어요"]')!;
  await act(async()=>otherAction.click());expect(host.textContent).toContain(pendingConfirmation);
  await act(async()=>done({...other,stats:{...other.stats!,liked:true}}));expect(host.textContent).toContain(pendingConfirmation);
  expect(api.action).toHaveBeenCalledTimes(2);
});
it("keeps uncertainty when the later matching read itself fails",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  vi.mocked(api.load).mockRejectedValueOnce(new Error("읽기 실패"));await click("새로고침");
  expect(host.textContent).toContain(pendingConfirmation);expect(host.textContent).toContain("읽기 실패");
});
it("does not let a late old-account read clear a new-account warning",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click("새로고침");
  vi.mocked(api.load).mockResolvedValue({...page([{...post,text:"새 계정의 글"}]),generation:2});
  await click("계정");await click("@bob");await click("마음에 들어요");
  await act(async()=>done(page([{...post,stats:{...post.stats!,liked:true}}])));
  expect(host.querySelector(".x-timeline")).toHaveTextContent("새 계정의 글");expect(host.textContent).toContain(pendingConfirmation);
});
it("rejects old-epoch read evidence but can resolve from a new same-owner resumed read",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("마음에 들어요");
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click("새로고침");
  await render(api,false);
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,id:"555555555",text:"재개 후 다른 행"}]));await render(api,true);
  await act(async()=>done(page([{...post,stats:{...post.stats!,liked:true}}])));
  expect(host.textContent).toContain(pendingConfirmation);expect(host.querySelector(".x-timeline")).toHaveTextContent("재개 후 다른 행");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats:{...post.stats!,liked:false}}]));await click("새로고침");
  expect(host.textContent).not.toContain(pendingConfirmation);expect(api.action).toHaveBeenCalledTimes(1);
});
it.each([true,false])("uses an identified full quote, never a linkless quote preview, as fresh state evidence (%s)",async identified=>{
  const api=mock(),outer={...post,id:"555555555",handle:"outer",name:"Outer",quote:post};
  vi.mocked(api.load).mockResolvedValue(page([outer]));api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  await act(async()=>host.querySelector<HTMLButtonElement>('.x-quote [aria-label="마음에 들어요"]')!.click());
  vi.mocked(api.load).mockResolvedValueOnce(page([identified
    ?{...outer,quote:{...post,stats:{...post.stats!,liked:true}}}
    :{...outer,quote:null,quotePreview:{handle:post.handle,name:post.name,text:post.text,time:post.time,media:[],video:false}}]));
  await click("새로고침");expect(host.textContent?.includes(pendingConfirmation)).toBe(!identified);
});
it("resolves a matching warning without remounting retained detail media or resetting either scroll",async()=>{
  const api=mock(),mediaPost={...post,media:["https://pbs.twimg.com/media/A.jpg"]};
  vi.mocked(api.load).mockResolvedValue(page([mediaPost]));api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);await click("Artist 게시물 미리보기");
  const left=host.querySelector<HTMLDivElement>(".x-scroll")!,right=host.querySelector<HTMLDivElement>(".x-preview-scroll")!;
  const media=right.querySelector(".x-images");expect(media).not.toBeNull();
  await act(async()=>{left.scrollTop=420;left.dispatchEvent(new Event("scroll",{bubbles:true}));right.scrollTop=275;right.dispatchEvent(new Event("scroll",{bubbles:true}));});
  await click("마음에 들어요");vi.mocked(api.load).mockResolvedValueOnce(page([{...mediaPost,stats:{...mediaPost.stats!,liked:true}}]));await click("새로고침");
  expect(host.textContent).not.toContain(pendingConfirmation);expect(host.querySelector(".x-preview.has-selection")).not.toBeNull();
  expect(host.querySelector(".x-preview-scroll .x-images")).toBe(media);expect(left.scrollTop).toBe(420);expect(right.scrollTop).toBe(275);
});
it.each(["like","bookmark"] as const)("recovers only the warned %s fields in an already open detail",async kind=>{
  const api=mock(),mediaPost={...post,media:["https://pbs.twimg.com/media/A.jpg"]};
  vi.mocked(api.load).mockResolvedValue(page([mediaPost]));api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  const retained={...mediaPost,text:"보존할 상세 본문",stats:{
    ...post.stats!,replies:"12",reposts:"8",views:"999",likes:"41",bookmarks:"11",
    liked:kind!=="like",bookmarked:kind!=="bookmark",canLike:kind!=="like",canBookmark:kind!=="bookmark",
  }};
  vi.mocked(api.load).mockResolvedValueOnce(page([retained]));await click("Artist 게시물 미리보기");
  const left=host.querySelector<HTMLDivElement>(".x-scroll")!,right=host.querySelector<HTMLDivElement>(".x-preview-scroll")!;
  const media=right.querySelector(".x-images"),detailPost=right.querySelector("article");
  await act(async()=>{left.scrollTop=420;left.dispatchEvent(new Event("scroll",{bubbles:true}));right.scrollTop=275;right.dispatchEvent(new Event("scroll",{bubbles:true}));});
  const label=kind==="like"?"마음에 들어요":"게시물 북마크";
  await click(label);
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,text:"새 피드 본문",stats:{
    ...post.stats!,liked:kind==="like",bookmarked:kind==="bookmark",likes:"7",bookmarks:"9",
  }}]));
  await click("새로고침");
  const recovered=right.querySelector<HTMLButtonElement>(kind==="like"?'[aria-label="마음에 들어요"]':'[aria-label="게시물 북마크"]')!;
  const other=right.querySelector<HTMLButtonElement>(kind==="like"?'[aria-label="게시물 북마크"]':'[aria-label="마음에 들어요"]')!;
  expect(recovered).toHaveAttribute("aria-pressed","true");expect(recovered).not.toBeDisabled();
  expect(other).toHaveAttribute("aria-pressed","true");expect(other).not.toBeDisabled();
  expect(right.querySelector('[aria-label="마음에 들어요"]')).toHaveTextContent(kind==="like"?"7":"41");
  expect(right.querySelector('[aria-label="답글 보기"]')).toHaveTextContent("12");expect(right.querySelector(".x-repost-count")).toHaveTextContent("8");expect(right.querySelector(".x-view-count")).toHaveTextContent("999");
  expect(right).toHaveTextContent("보존할 상세 본문");expect(right).not.toHaveTextContent("새 피드 본문");
  expect(right.querySelector("article")).toBe(detailPost);expect(right.querySelector(".x-images")).toBe(media);
  expect(left.scrollTop).toBe(420);expect(right.scrollTop).toBe(275);expect(host.textContent).not.toContain(pendingConfirmation);
  expect(api.action).toHaveBeenCalledTimes(1);
});
it("repairs only the warned action inside retained full quotes without replacing quote content",async()=>{
  const api=mock(),quoted={...post,text:"보존할 인용 본문",media:["https://pbs.twimg.com/media/A.jpg"],stats:{...post.stats!,bookmarked:true,likes:"41"}};
  const outer={...post,id:"555555555",handle:"outer",name:"Outer",text:"바깥 본문",quote:quoted};
  vi.mocked(api.load).mockResolvedValue(page([outer]));api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  await act(async()=>host.querySelector<HTMLButtonElement>('.x-quote [aria-label="마음에 들어요"]')!.click());
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,text:"새 관찰 본문",stats:{...post.stats!,liked:true,likes:"7",bookmarked:false}}]));await click("팔로잉");
  expect(host.textContent).not.toContain(pendingConfirmation);await click("추천");
  const quote=host.querySelector(".x-quote")!;
  expect(quote.querySelector('[aria-label="마음에 들어요"]')).toHaveAttribute("aria-pressed","true");expect(quote.querySelector('[aria-label="마음에 들어요"]')).toHaveTextContent("7");
  expect(quote.querySelector('[aria-label="게시물 북마크"]')).toHaveAttribute("aria-pressed","true");
  expect(quote).toHaveTextContent("보존할 인용 본문");expect(quote).not.toHaveTextContent("새 관찰 본문");expect(quote.querySelector(".x-images")).not.toBeNull();
  expect(host.querySelector('.x-timeline>article>.x-actions [aria-label="마음에 들어요"]')).toHaveAttribute("aria-pressed","false");
  expect(api.load).toHaveBeenCalledTimes(2);expect(api.action).toHaveBeenCalledTimes(1);
});
it.each(["author","generation"] as const)("does not synchronize a cached detail from another %s",async mismatch=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  vi.mocked(api.load).mockResolvedValueOnce({...page([{...post,handle:mismatch==="author"?"other":post.handle}]),generation:mismatch==="generation"?2:1});
  await click("Artist 게시물 미리보기");await click("마음에 들어요");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats:{...post.stats!,liked:true,likes:"7"}}]));await click("새로고침");
  expect(host.querySelector('.x-timeline [aria-label="마음에 들어요"]')).toHaveAttribute("aria-pressed","true");
  expect(host.querySelector('.x-thread [aria-label="마음에 들어요"]')).toHaveAttribute("aria-pressed","false");
  expect(host.querySelector('.x-thread [aria-label="마음에 들어요"]')).toHaveTextContent("4");expect(host.textContent).not.toContain(pendingConfirmation);
});
it("does not invent unobserved stats in a cached detail with no stats",async()=>{
  const api=mock();api.action=vi.fn().mockRejectedValue(new Error(pendingConfirmation));await render(api);
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats:undefined}]));await click("Artist 게시물 미리보기");await click("마음에 들어요");
  vi.mocked(api.load).mockResolvedValueOnce(page([{...post,stats:{...post.stats!,liked:true,likes:"7"}}]));await click("새로고침");
  const action=host.querySelector<HTMLButtonElement>('.x-thread [aria-label="마음에 들어요"]')!;
  expect(action).toBeDisabled();expect(action).not.toHaveAttribute("aria-pressed");expect(action).not.toHaveTextContent("7");
  expect(host.querySelector('.x-timeline [aria-label="마음에 들어요"]')).toHaveAttribute("aria-pressed","true");expect(host.textContent).not.toContain(pendingConfirmation);
});
it("revalidates an expired read cursor once and retains same-account loaded posts",async()=>{
  const api=mock();await render(api);
  vi.mocked(api.load).mockRejectedValueOnce("X 탐색 상태가 변경되었거나 만료되었습니다. 새로고침해 주세요.").mockResolvedValueOnce(page([{...post,id:"555555555",text:"새 게시물"}]));
  await click("더 보기");expect(api.load).toHaveBeenCalledTimes(3);expect(api.load).toHaveBeenLastCalledWith("foryou",null,expect.any(String));
  expect(host.querySelectorAll('.x-timeline>article')).toHaveLength(2);expect(host.querySelector('[role="alert"]')).toBeNull();
});
it("does not loop when fresh validation also fails or retain another generation",async()=>{
  const api=mock();await render(api);vi.mocked(api.load).mockRejectedValue("X 탐색 상태가 변경되었거나 만료되었습니다. 새로고침해 주세요.");
  await click("더 보기");expect(api.load).toHaveBeenCalledTimes(3);expect(host.querySelector('[role="alert"]')).not.toBeNull();
  vi.mocked(api.load).mockResolvedValueOnce({...page([{...post,id:"555555555",text:"다른 계정"}]),generation:2});await click("다시 시도");
  expect(host.querySelectorAll('.x-timeline>article')).toHaveLength(1);expect(host.querySelector('.x-timeline')).not.toHaveTextContent("풍경 사진");
});
it("keeps errors explicit and ignores a social result after account invalidation",async()=>{
  const api=mock();let done!:(p:XPost)=>void;api.action=vi.fn().mockReturnValue(new Promise(r=>done=r));await render(api);await click("마음에 들어요");await render(api,true,true);await act(async()=>done({...post,stats:{...post.stats!,liked:true}}));expect(host.querySelector("article")).toBeNull();
});
it("reloads a revoked reader after resuming without claiming an expired cursor works",async()=>{
  const api=mock();await render(api);await render(api,false);await render(api,true);expect(api.load).toHaveBeenCalledTimes(2);
});
it("cancels an old feed when switching tabs and does not let its late result replace the new view",async()=>{
  const api=mock();let done!:(p:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(r=>done=r));await render(api);await click("북마크");expect(api.cancel).toHaveBeenCalledTimes(1);await act(async()=>done(page([{...post,text:"취소된 응답"}])));expect(host.textContent).not.toContain("취소된 응답");expect(button("북마크")).toHaveAttribute("aria-pressed","true");
});
it("ends an unresponsive request with an explicit timeout instead of an infinite skeleton",async()=>{
  vi.useFakeTimers();const api=mock();vi.mocked(api.load).mockReturnValue(new Promise(()=>{}));await render(api);await act(async()=>vi.advanceTimersByTimeAsync(35001));expect(host.querySelector('[role="alert"]')).toHaveTextContent("응답 대기 시간이 길어");expect(api.cancel).toHaveBeenCalled();expect(host.querySelector('[aria-label="피드 불러오는 중"]')).toBeNull();
});
it("waits for the StrictMode cleanup suspension before starting its surviving reader",async()=>{
  const api=mock();let resume!:()=>void;
  vi.mocked(api.suspend).mockReturnValueOnce(new Promise(r=>resume=r));
  await act(async()=>root.render(<StrictMode><XWorkspace api={api} active privacyMode={false} runtime="browser-mock" onNotice={vi.fn()}/></StrictMode>));
  expect(api.load).not.toHaveBeenCalled();
  await act(async()=>resume());
  expect(api.load).toHaveBeenCalledTimes(1);expect(host.querySelector("article")).not.toBeNull();
});
it("waits for an in-flight native suspension on immediate re-entry",async()=>{
  const api=mock();await render(api);let resume!:()=>void;
  vi.mocked(api.suspend).mockReturnValueOnce(new Promise(r=>resume=r));
  await render(api,false);await render(api,true);expect(api.load).toHaveBeenCalledTimes(1);
  await act(async()=>resume());expect(api.load).toHaveBeenCalledTimes(2);
});

// 2026-10-18 completion contract: read-only navigation/focus/session assertions.
// These fixtures do not perform real account operations or establish desktop acceptance.
const key=(target:Element,key:string,shiftKey=false)=>act(async()=>target.dispatchEvent(new KeyboardEvent('keydown',{key,shiftKey,bubbles:true,cancelable:true})));
const search=async(value:string)=>{
  const input=host.querySelector<HTMLInputElement>('[aria-label="불러온 게시물 검색"]')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,value);input.dispatchEvent(new Event('input',{bubbles:true}));});
};
it('closes only the enlarged image on Escape and restores its trigger, not the underlying preview',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,media:['https://pbs.twimg.com/media/A.jpg']}]));await render(api);
  await click('Artist 게시물 미리보기');
  const trigger=host.querySelector<HTMLButtonElement>('.x-thread [aria-label="이미지 1 크게 보기"]')!;
  trigger.focus();await act(async()=>trigger.click());
  expect(document.activeElement).toBe(button('이미지 닫기'));
  await key(button('이미지 닫기'),'Escape');
  expect(host.querySelector('[role="dialog"]')).toBeNull();expect(document.activeElement).toBe(trigger);
  expect(host.querySelector('.x-preview.has-selection')).not.toBeNull();
});
it('keeps enlarged-image keyboard navigation and Tab focus inside its own controls',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,media:['https://pbs.twimg.com/media/A.jpg','https://pbs.twimg.com/media/B.jpg']}]));await render(api);await click('이미지 1 크게 보기');
  await key(button('이미지 닫기'),'Tab',true);expect(document.activeElement).toBe(button('다음 이미지'));
  await key(button('다음 이미지'),'Tab');expect(document.activeElement).toBe(button('이미지 닫기'));
  await key(button('이미지 닫기'),'ArrowLeft');expect(host.querySelector('.x-image-viewer header')).toHaveTextContent('2 / 2');
  await key(button('이미지 닫기'),'ArrowRight');expect(host.querySelector('.x-image-viewer header')).toHaveTextContent('1 / 2');
  expect(api.load).toHaveBeenCalledTimes(1);
});
it('discards the enlarged image across privacy and inactive transitions',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,media:['https://pbs.twimg.com/media/A.jpg']}]));await render(api);await click('이미지 1 크게 보기');
  await render(api,false);expect(host.querySelector('[role="dialog"]')).toBeNull();
  await render(api,true);expect(host.querySelector('[role="dialog"]')).toBeNull();
  await click('이미지 1 크게 보기');await render(api,true,true);expect(host.querySelector('[role="dialog"]')).toBeNull();expect(host.querySelector('article')).toBeNull();
});
it('clears an enlarged old-account image when an already-running page request reports signed out',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,media:['https://pbs.twimg.com/media/A.jpg']}]));await render(api);
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click('더 보기');await click('이미지 1 크게 보기');
  await act(async()=>done({...page([]),status:'signedOut',cursor:null}));
  expect(host.querySelector('[role="alert"]')).toHaveTextContent('X 로그인이 필요합니다. 계정 메뉴에서 현재 계정 정보 복구를 선택해 주세요.');expect(host.querySelector('[role="dialog"]')).toBeNull();expect(host.querySelector('article')).toBeNull();
});
it('restores search, media filtering, both scroll positions and the selected preview after author navigation',async()=>{
  const api=mock();vi.mocked(api.load).mockImplementation(async feed=>feed.startsWith('profile:')?page([{...post,text:'작성자의 다른 게시물',id:'555555555'}]):page([{...post,media:['https://pbs.twimg.com/media/A.jpg']}]));await render(api);
  await search('풍경');await click('미디어');
  const left=host.querySelector<HTMLDivElement>('.x-scroll')!;await act(async()=>{left.scrollTop=420;left.dispatchEvent(new Event('scroll',{bubbles:true}));});
  await click('Artist 게시물 미리보기');
  const right=host.querySelector<HTMLDivElement>('.x-preview-scroll')!;await act(async()=>{right.scrollTop=275;right.dispatchEvent(new Event('scroll',{bubbles:true}));});
  const author=host.querySelector<HTMLButtonElement>('.x-thread [aria-label="Artist 프로필 보기"]')!;await act(async()=>author.click());
  expect(host.querySelector<HTMLInputElement>('[aria-label="불러온 게시물 검색"]')!.value).toBe('');expect(button('미디어')).toHaveAttribute('aria-pressed','false');
  await click('이전 피드로');
  expect(host.querySelector<HTMLInputElement>('[aria-label="불러온 게시물 검색"]')!.value).toBe('풍경');expect(button('미디어')).toHaveAttribute('aria-pressed','true');
  expect(host.querySelector<HTMLDivElement>('.x-scroll')!.scrollTop).toBe(420);expect(host.querySelector('.x-preview.has-selection')).not.toBeNull();
  expect(host.querySelector<HTMLDivElement>('.x-preview-scroll')!.scrollTop).toBe(275);
});
it('keeps unknown thread entries unclassified through confirmed reaction updates',async()=>{
  const reply={...post,id:'555555555',text:'명시적 답글',relation:'reply' as const};
  const api=mock();await render(api);vi.mocked(api.load).mockResolvedValueOnce(page([post,reply,{...post,id:'666666666',text:'명시되지 않은 게시물'}]));await click('Artist 게시물 미리보기');
  api.action=vi.fn().mockResolvedValue({...reply,relation:undefined,stats:{...reply.stats!,liked:true}});
  const reaction=host.querySelector<HTMLButtonElement>('section[aria-label="답글"] [aria-label="마음에 들어요"]')!;await act(async()=>reaction.click());
  expect(host.querySelector('section[aria-label="답글"]')).toHaveTextContent('명시적 답글');expect(host.querySelector('section[aria-label="분류 미확인 게시물"]')).toHaveTextContent('명시되지 않은 게시물');
  expect(host.querySelector('section[aria-label="분류 미확인 게시물"]')).not.toHaveTextContent('명시적 답글');
});
it('does not restore an old account profile or its navigation history after switching accounts',async()=>{
  const api=mock();await render(api);
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click('Artist 프로필 보기');
  vi.mocked(api.load).mockResolvedValue({...page([{...post,id:'555555555',handle:'bob',name:'Bob',text:'두 번째 계정 피드'}]),generation:2,communities:[]});
  await click('계정');await click('@bob');await act(async()=>done(page([{...post,text:'이전 계정 프로필'}])));
  expect(api.select).toHaveBeenCalledWith('b');expect(host.querySelector('.x-timeline')).toHaveTextContent('두 번째 계정 피드');expect(host.textContent).not.toContain('이전 계정 프로필');
  expect(host.querySelector('.x-profile-heading')).toBeNull();expect(host.querySelector('.x-preview.has-selection')).toBeNull();expect(host.querySelector('[role="dialog"]')).toBeNull();
  await key(button('추천'),'Escape');expect(button('추천')).toHaveAttribute('aria-pressed','true');expect(host.querySelector('.x-profile-heading')).toBeNull();
});
it('keeps existing replies stable while appending and shows progress only at the thread bottom',async()=>{
  const api=mock();await render(api);vi.mocked(api.load).mockResolvedValueOnce(page([post,{...post,id:'555555555',relation:'reply',text:'기존 답글'}]));await click('Artist 게시물 미리보기');
  const thread=host.querySelector<HTMLDivElement>('.x-preview-scroll')!;
  const reply=thread.querySelector('section[aria-label="답글"] article');
  await act(async()=>{thread.scrollTop=275;thread.dispatchEvent(new Event('scroll',{bubbles:true}));});
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await click('답글·관련 게시물 더 보기');
  expect(thread.querySelector('.x-skeleton')).toBeNull();expect(thread.querySelector('.x-more [role="status"]')).toHaveTextContent('더 불러오는 중');
  expect(thread.querySelector('section[aria-label="답글"] article')).toBe(reply);expect(thread.scrollTop).toBe(275);
  await act(async()=>done({...page([{...post,id:'666666666',relation:'reply',text:'추가 답글'}]),cursor:null}));
  expect(thread.querySelector('.x-more [role="status"]')).toBeNull();expect(thread.querySelector('section[aria-label="답글"]')).toHaveTextContent('기존 답글');expect(thread.querySelector('section[aria-label="답글"]')).toHaveTextContent('추가 답글');
});
it.each(['unknown',null,undefined] as const)('does not demote a proven reply when same-generation append lacks relation evidence (%s)',async relation=>{
  const api=mock();await render(api);const reply={...post,id:'555555555',relation:'reply' as const,text:'확인한 답글'};
  vi.mocked(api.load).mockResolvedValueOnce(page([post,reply]));await click('Artist 게시물 미리보기');
  vi.mocked(api.load).mockResolvedValueOnce({...page([{...reply,relation,text:'갱신된 답글'}]),cursor:null});await click('답글·관련 게시물 더 보기');
  expect(host.querySelector('section[aria-label="답글"]')).toHaveTextContent('갱신된 답글');expect(host.querySelector('section[aria-label="분류 미확인 게시물"]')).toBeNull();
});
it('accepts newly explicit relation evidence while appending a previously unknown post',async()=>{
  const api=mock();await render(api);const other={...post,id:'555555555',relation:'unknown' as const,text:'분류 확인 전'};
  vi.mocked(api.load).mockResolvedValueOnce(page([post,other]));await click('Artist 게시물 미리보기');
  vi.mocked(api.load).mockResolvedValueOnce({...page([{...other,relation:'related',text:'확인된 관련 게시물'}]),cursor:null});await click('답글·관련 게시물 더 보기');
  expect(host.querySelector('section[aria-label="관련 게시물"]')).toHaveTextContent('확인된 관련 게시물');expect(host.querySelector('section[aria-label="분류 미확인 게시물"]')).toBeNull();
});
it('lets fresh explicit relation evidence replace an older explicit classification',async()=>{
  const api=mock();await render(api);const other={...post,id:'555555555',relation:'reply' as const,text:'이전 분류'};
  vi.mocked(api.load).mockResolvedValueOnce(page([post,other]));await click('Artist 게시물 미리보기');
  vi.mocked(api.load).mockResolvedValueOnce({...page([{...other,relation:'related',text:'새롭게 확인된 관련 영역'}]),cursor:null});await click('답글·관련 게시물 더 보기');
  expect(host.querySelector('section[aria-label="관련 게시물"]')).toHaveTextContent('새롭게 확인된 관련 영역');expect(host.querySelector('section[aria-label="답글"]')).toBeNull();
});
it('keeps an enlarged linkless quoted-video poster as a valid one-image selection',async()=>{
  const api=mock();vi.mocked(api.load).mockResolvedValue(page([{...post,quotePreview:{handle:'quoted',name:'Quoted',text:'인용된 영상',time:'',media:['https://pbs.twimg.com/tweet_video_thumb/Poster.jpg'],video:true}}]));await render(api);
  await click('이미지 1 크게 보기');expect(host.querySelector('.x-image-viewer header')).toHaveTextContent('1 / 1');
});
it('clears retained owner views before presenting a changed account discovered during resume',async()=>{
  const api=mock();await render(api);await render(api,false);
  vi.mocked(api.status).mockResolvedValueOnce({generation:2,status:'signedIn',open:false,active:'b',accounts:[{id:'a',handle:'alice'},{id:'b',handle:'bob'}]});
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));await render(api,true);
  try{expect(button('계정')).toHaveTextContent('@bob');expect(host.querySelector('article')).toBeNull();expect(host.textContent).not.toContain('풍경 사진');}
  finally{await act(async()=>done({...page([{...post,id:'555555555',name:'Bob',text:'새 계정 피드'}]),generation:2}));}
});
it('automatically appends the right-hand thread only once per in-flight request and stops after closing it',async()=>{
  const observed:{root:Element|null;callback:IntersectionObserverCallback;target:Element|null;disconnected:boolean}[]=[];
  vi.stubGlobal('IntersectionObserver',class {
    entry:typeof observed[number];
    constructor(callback:IntersectionObserverCallback,options?:IntersectionObserverInit){this.entry={root:options?.root instanceof Element?options.root:null,callback,target:null,disconnected:false};observed.push(this.entry);}
    observe(target:Element){this.entry.target=target;}unobserve(){}disconnect(){this.entry.disconnected=true;}takeRecords(){return [];}
  });
  const api=mock();await render(api);await click('Artist 게시물 미리보기');
  const observer=[...observed].reverse().find(item=>item.root?.classList.contains('x-preview-scroll'))!;
  expect(observer.target?.classList.contains('x-more')).toBe(true);
  let done!:(value:XPage)=>void;vi.mocked(api.load).mockReturnValueOnce(new Promise(resolve=>done=resolve));
  const notify=()=>observer.callback([{isIntersecting:true,target:observer.target!} as IntersectionObserverEntry],{} as IntersectionObserver);
  await act(async()=>{notify();notify();});expect(api.load).toHaveBeenCalledTimes(3);expect(api.load).toHaveBeenLastCalledWith('post:123456789','cursor',expect.any(String));
  await act(async()=>done({...page([post,{...post,id:'555555555',relation:'reply',text:'추가 답글'}]),cursor:null}));
  expect(host.querySelector('section[aria-label="답글"]')).toHaveTextContent('추가 답글');expect(observer.disconnected).toBe(true);
  await click('미리보기 닫기');expect(observed.filter(item=>item.root?.classList.contains('x-preview-scroll')&&!item.disconnected)).toHaveLength(0);
});
