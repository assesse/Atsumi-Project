import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { downloadPreviewItems, nextDownloadPreviewIndex } from "./downloadPreviewNavigation";

const album = (id: number, state: NonNullable<Gallery["download"]>["state"] = "completed"): Gallery => ({
  ...mockGalleries[0]!, id: galleryId(id), pages: 12, download: { entryId: "local-" + id, state },
});
describe("download reading order", () => {
  it("keeps displayed folder and album order, includes collapsed folders and skips empty/unreadable folders", () => {
    const [a,b,c,d,e] = [album(1), album(2), album(3), album(4,"hashing"), album(5,"quarantined")];
    const items = downloadPreviewItems([c!,b!,a!,d!,e!], [
      { key: "a", label: "Artist A", items: [b!,a!,d!] },
      { key: "empty", label: "Pending", items: [e!] },
      { key: "c", label: "Artist C", items: [c!] },
    ]);
    expect(items.map(item=>item.galleryId)).toEqual([2,1,3]);
    expect(items.map(item=>item.folder)).toEqual([
      { index:1,total:2,label:"Artist A" },{ index:2,total:2,label:"Artist A" },{ index:1,total:1,label:"Artist C" },
    ]);
    const galleries = new Map([a!,b!,c!].map(g=>[g.id,g]));
    expect(nextDownloadPreviewIndex(items,1,1,galleries)).toBe(2);
    expect(nextDownloadPreviewIndex(items,2,-1,galleries)).toBe(1);
    expect(nextDownloadPreviewIndex(items,0,-1,galleries)).toBeNull();
    expect(nextDownloadPreviewIndex(items,2,1,galleries)).toBeNull();
  });
  it("keeps every flat-list page in order without adding an artist counter", () => {
    const albums = Array.from({length:151}, (_,i)=>album(i+1));
    const items=downloadPreviewItems(albums);
    expect(items).toHaveLength(151);
    expect(items.every(item=>item.folder===undefined)).toBe(true);
    expect(nextDownloadPreviewIndex(items,49,1,new Map(albums.map(g=>[g.id,g])))).toBe(50);
  });
  it("does not duplicate entries or navigate into removed, excluded or unfinished files", () => {
    const a=album(1), b=album(2), c=album(3), d=album(4), noPages={...album(5),pages:0};
    const items=downloadPreviewItems([a,a,b,c,d,noPages]);
    const current=new Map([a,album(2,"quarantined"),d].map(g=>[g.id,g]));
    expect(items.map(item=>item.galleryId)).toEqual([1,2,3,4]);
    expect(nextDownloadPreviewIndex(items,0,1,current)).toBe(3);
    expect(nextDownloadPreviewIndex(items,3,-1,current)).toBe(0);
    expect(nextDownloadPreviewIndex(items,-1,1,current)).toBeNull();
  });
  it("retains the opened reading snapshot when background sorting or completion changes", () => {
    const a=album(1),b=album(2),source=[a,b];
    const items=downloadPreviewItems(source,[{key:"a",label:"A",items:source}]);
    source.reverse();source.push(album(3));
    expect(items.map(item=>item.galleryId)).toEqual([1,2]);
    expect(items[1]?.folder).toEqual({index:2,total:2,label:"A"});
  });
});
