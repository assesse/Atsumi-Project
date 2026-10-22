import { beforeEach,expect,it,vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { createXSurfaceApi } from "./api";
vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn().mockResolvedValue(undefined)}));
beforeEach(()=>vi.clearAllMocks());
it("passes opaque preview ids with current generation through closed native commands",async()=>{
  const api=createXSurfaceApi("tauri");
  await api.selectPreview(9,"preview-3");expect(invoke).toHaveBeenLastCalledWith("x_surface_select_preview",{generation:9,previewId:"preview-3"});
  await api.closePreview(10,"preview-2");expect(invoke).toHaveBeenLastCalledWith("x_surface_close_preview",{generation:10,previewId:"preview-2"});
  await api.closePreview(11);expect(invoke).toHaveBeenLastCalledWith("x_surface_close_preview",{generation:11});
  await api.dismissMenus(12);expect(invoke).toHaveBeenLastCalledWith("x_surface_dismiss_menus",{generation:12});
});
it("never opens an external tab or pretends native previews exist in browser-mock runtime",async()=>{
  const api=createXSurfaceApi("browser-mock");
  await expect(api.selectPreview(1,"preview-1")).rejects.toThrow("데스크톱");
  await expect(api.closePreview(1,"preview-1")).rejects.toThrow("데스크톱");
  await api.dismissMenus(1);expect(invoke).not.toHaveBeenCalled();
});
it("distinguishes ordinary document concealment from privacy and account invalidation",async()=>{
  const api=createXSurfaceApi("tauri");
  await api.suspend(true);expect(invoke).toHaveBeenLastCalledWith("x_surface_suspend");
  await api.suspend(false);expect(invoke).toHaveBeenLastCalledWith("x_feed_suspend");
  await api.suspend();expect(invoke).toHaveBeenLastCalledWith("x_feed_suspend");
  vi.mocked(invoke).mockClear();await createXSurfaceApi("browser-mock").suspend(true);expect(invoke).not.toHaveBeenCalled();
});
