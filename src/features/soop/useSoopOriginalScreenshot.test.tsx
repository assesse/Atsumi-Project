import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SoopOriginalApi, SoopScreenshotSnapshot } from '../../api/soopOriginal';
import { useSoopOriginalScreenshot } from './useSoopOriginalScreenshot';
const saved={item:{id:'capture-one'},preview:'data:image/png;base64,fixture'} as SoopScreenshotSnapshot;
const api=()=>({screenshot:vi.fn().mockResolvedValue(saved),undoScreenshot:vi.fn().mockResolvedValue(undefined)}) as unknown as SoopOriginalApi;
let root:Root,container:HTMLDivElement;
beforeEach(()=>{container=document.createElement('div');document.body.append(container);root=createRoot(container);});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();vi.restoreAllMocks();});
async function setup(bridge:SoopOriginalApi,enabled=true,token='viewer'){
  const result={current:null as unknown as ReturnType<typeof useSoopOriginalScreenshot>};
  function Harness(props:{enabled:boolean;token:string}){result.current=useSoopOriginalScreenshot({api:bridge,...props});return null;}
  const rerender=async(props:{enabled:boolean;token:string})=>act(async()=>root.render(<Harness {...props}/>));
  await rerender({enabled,token});return{result,rerender};
}
describe('SOOP native screenshot lifecycle',()=>{
  it('shows the saved original frame and undoes exactly its own item',async()=>{
    const bridge=api();const {result}=await setup(bridge);
    await act(()=>result.current.save());expect(result.current.preview?.url).toBe(saved.preview);expect(bridge.screenshot).toHaveBeenCalledWith('viewer',expect.any(String));
    await act(()=>result.current.undo());expect(bridge.undoScreenshot).toHaveBeenCalledWith('viewer','capture-one');expect(result.current.preview).toBeNull();
  });
  it('does not capture a hidden/private pane and hides an in-flight preview after its owner changes',async()=>{
    const bridge=api();let resolve!:(value:SoopScreenshotSnapshot)=>void;vi.mocked(bridge.screenshot).mockReturnValue(new Promise(done=>resolve=done));
    const {result,rerender}=await setup(bridge,false,'viewer-a');
    await act(()=>result.current.save());expect(bridge.screenshot).not.toHaveBeenCalled();await rerender({enabled:true,token:'viewer-a'});
    act(()=>{void result.current.save();});expect(result.current.busy).toBe(true);await rerender({enabled:true,token:'viewer-b'});
    await act(async()=>resolve(saved));expect(result.current.preview).toBeNull();expect(result.current.busy).toBe(false);
  });
  it('coalesces duplicate capture intent and reports a failed undo without pretending deletion',async()=>{
    const bridge=api();let resolve!:(value:SoopScreenshotSnapshot)=>void;vi.mocked(bridge.screenshot).mockReturnValue(new Promise(done=>resolve=done));
    vi.mocked(bridge.undoScreenshot).mockRejectedValue('denied');
    const {result}=await setup(bridge);
    act(()=>{void result.current.save();void result.current.save();});expect(bridge.screenshot).toHaveBeenCalledOnce();
    await act(async()=>resolve(saved));await act(()=>result.current.undo());
    expect(result.current.preview?.error).toContain('취소를 확인하지 못했습니다');
    expect(result.current.preview?.id).toBe('capture-one');
  });
});
