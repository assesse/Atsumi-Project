import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { XPreviewTabs } from "./XPreviewTabs";
import type { XPreviewTab } from "./api";
let host:HTMLDivElement,root:Root;
const tabs:XPreviewTab[]=Array.from({length:3},(_,index)=>({id:`tab-${index}`,path:`/author/status/12345${index}`,title:`@author · 게시물 ${index}`,state:"ready"}));
const select=vi.fn(),close=vi.fn();
const render=(items=tabs,activeId="tab-0",disabled=false)=>act(async()=>root.render(<XPreviewTabs tabs={items} activeId={activeId} disabled={disabled} panelId="preview" onSelect={select} onClose={close}/>));
const tab=(index:number)=>host.querySelectorAll<HTMLButtonElement>('[role=tab]')[index]!;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.clearAllMocks();vi.restoreAllMocks();vi.unstubAllGlobals();});
it("renders independent accessible tab and centered-close controls without nesting buttons",async()=>{
  await render();
  expect(host.querySelector('[role=tablist]')).toHaveAttribute("aria-label","X 미리보기 탭");
  expect(host.querySelectorAll('[role=tab]')).toHaveLength(3);expect(host.querySelector("button button")).toBeNull();
  expect(tab(0)).toHaveAttribute("aria-selected","true");expect(tab(0)).toHaveAttribute("tabindex","0");
  expect(tab(1)).toHaveAttribute("tabindex","-1");expect(tab(1)).toHaveAttribute("aria-controls","preview");
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="@author · 게시물 2 닫기"]')!.click());
  expect(close).toHaveBeenCalledExactlyOnceWith("tab-2");expect(select).not.toHaveBeenCalled();
});
it("uses manual keyboard activation and leaves modified shortcuts alone",async()=>{
  await render();tab(0).focus();
  const key=(value:string,options:KeyboardEventInit={})=>act(async()=>document.activeElement?.dispatchEvent(new KeyboardEvent("keydown",{key:value,bubbles:true,cancelable:true,...options})));
  await key("ArrowRight");expect(document.activeElement).toBe(tab(1));expect(select).not.toHaveBeenCalled();
  await key("End");expect(document.activeElement).toBe(tab(2));
  await key("ArrowRight");expect(document.activeElement).toBe(tab(0));
  await key("ArrowLeft");expect(document.activeElement).toBe(tab(2));
  await key("Home");expect(document.activeElement).toBe(tab(0));
  await key("ArrowRight",{ctrlKey:true});expect(document.activeElement).toBe(tab(0));
  await key("End",{isComposing:true});expect(document.activeElement).toBe(tab(0));
  await act(async()=>tab(1).click());expect(select).toHaveBeenCalledExactlyOnceWith("tab-1");
});
it("scrolls the newly selected tab into view without stealing native or external focus",async()=>{
  const scroll=vi.fn();Object.defineProperty(HTMLElement.prototype,"scrollIntoView",{value:scroll,configurable:true});
  const external=document.createElement("button");document.body.append(external);external.focus();
  try{
    await render();scroll.mockClear();await render(tabs,"tab-2");
    expect(scroll).toHaveBeenCalledExactlyOnceWith({block:"nearest",inline:"nearest",behavior:"instant"});
    expect(document.activeElement).toBe(external);scroll.mockClear();
    await render(tabs.map(item=>({...item,state:"loading"})),"tab-2");
    expect(scroll).not.toHaveBeenCalled();
    await render([...tabs,{id:"background",path:"/another",title:"@another",state:"loading"}],"tab-2");
    expect(scroll).not.toHaveBeenCalled();expect(document.activeElement).toBe(external);
  }finally{external.remove();delete (HTMLElement.prototype as Partial<HTMLElement>).scrollIntoView;}
});
it("returns focus to the adjacent selected tab only after the focused tab was removed",async()=>{
  await render();host.querySelector<HTMLButtonElement>('[aria-label="@author · 게시물 0 닫기"]')!.focus();
  await render(tabs.slice(1),"tab-1");expect(document.activeElement).toBe(tab(0));
  expect(select).not.toHaveBeenCalled();expect(close).not.toHaveBeenCalled();
});
it("disables native tab mutations while a prior tab operation is pending",async()=>{
  await render(tabs,"tab-0",true);
  for(const button of host.querySelectorAll<HTMLButtonElement>("button")){expect(button).toBeDisabled();button.click();}
  expect(select).not.toHaveBeenCalled();expect(close).not.toHaveBeenCalled();
});
