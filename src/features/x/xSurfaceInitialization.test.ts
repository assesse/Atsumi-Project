import { expect, it, vi } from "vitest";
import native from "../../../src-tauri/src/x_surface.rs?raw";
import compat from "../../../src-tauri/src/x_surface_source_compat.rs?raw";
const files=import.meta.glob("../../../src-tauri/src/x_surface*.js",{eager:true,query:"?raw",import:"default"}) as Record<string,string>;
const source=(name:string)=>{
  const text=files["../../../src-tauri/src/"+name];if(!text)throw new Error("Missing injected source "+name);return text;
};
function initialization(feed:string){
  const config=JSON.stringify({nonce:"fixture",owner:"alice",feed,diagnostics:false,searchEnabled:false});
  // Read the real Rust order and separator: an ASI boundary can parse but throw.
  const parts=native.match(/let script = \[([\s\S]*?)\]\.join\(("(?:\\.|[^"\\])*")\);/);
  const original=compat.match(/pub\(super\) fn initialization[\s\S]*?\[([\s\S]*?)\]\s*\.join\(("(?:\\.|[^"\\])*")\)/);
  if(!parts||!original)throw new Error("Native initialization assembly changed");
  const base=[...original[1]!.matchAll(/include_str!\("([^"]+)"\)/g)].map(m=>source(m[1]!)).join(JSON.parse(original[2]!));
  const collectionCss=import.meta.glob("../../../src-tauri/src/x_surface_collection.css",{eager:true,query:"?raw",import:"default"}) as Record<string,string>;
  const scripts=[base,...[...parts[1]!.matchAll(/include_str!\("([^"]+\.js)"\)/g)].map(m=>source(m[1]!))];
  return scripts.join(JSON.parse(parts[2]!)).replace(/__ATSUMI_X_(?:SOURCE_COMPAT|ADS|ACTIONS|INPUT|SURFACE|PRESENTATION|VIDEO|COLLECTION)__/g,config).replace("__ATSUMI_X_COLLECTION_CSS__",JSON.stringify(Object.values(collectionCss)[0]));
}
it.each(["foryou","preview","likes","bookmarks"])("initializes every %s adapter in the actual native combined script",feed=>{
  const doc=new DOMParser().parseFromString("<body></body>","text/html"),timers:Function[]=[];
  const win:Record<string,unknown>={addEventListener:vi.fn(),chrome:{webview:{postMessage:vi.fn()}},getSelection:()=>({toString:()=>""})};
  win.top=win;
  class Observer{observe(){} disconnect(){}}
  const run=new Function("window","document","location","MutationObserver","setTimeout","clearTimeout",initialization(feed));
  expect(()=>run(win,doc,{protocol:"https:",hostname:"x.com",origin:"https://x.com",pathname:"/home",href:"https://x.com/home",port:""},Observer,(fn:Function)=>{timers.push(fn);return timers.length;},()=>{})).not.toThrow();
  for(const name of ["__atsumiXAds","__atsumiXActions","__atsumiXInput","__atsumiXSurface","__atsumiXVideo"]){
    expect(win[name],name).toBeDefined();
  }
  if(["likes","bookmarks"].includes(feed))expect(win.__atsumiXCollection).toBeDefined();
  if(feed==="preview")expect(win.__atsumiXPresentation).toBeDefined();
  else expect(win.__atsumiXRouteAdapter).toBeDefined();
});
