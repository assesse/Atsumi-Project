const fsName="node:fs",pathName="node:path",osName="node:os",childName="node:child_process",processName="node:process";
const fs=await import(fsName) as {existsSync(path:string):boolean;mkdtempSync(prefix:string):string;writeFileSync(path:string,content:string):void;realpathSync(path:string):string;rmSync(path:string,options:{recursive:boolean;force:boolean;maxRetries:number;retryDelay:number}):void};
const path=await import(pathName) as {join(...parts:string[]):string;relative(from:string,to:string):string;isAbsolute(path:string):boolean};
const {tmpdir}=await import(osName) as {tmpdir():string};
const {execFile}=await import(childName) as {execFile(file:string,args:string[],options:{timeout:number;maxBuffer:number;encoding:"utf8";windowsHide:boolean},callback:(error:Error|null,stdout:string,stderr:string)=>void):void};
const {env,execPath,cwd}=await import(processName) as {env:Record<string,string|undefined>;execPath:string;cwd():string};
export const layoutBrowser=[env.ATSUMI_TEST_BROWSER??"","C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe","C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"].find(candidate=>fs.existsSync(candidate));
export type LayoutScene={name:string;html:string};
export type LayoutResult={name:string;width:number;height:number;issues:string[]};
/** Native WebSocket and Event constructors run outside jsdom's DOM realm. */
export async function auditLayouts(scenes:LayoutScene[],css:string,width:number,height:number,group?:string):Promise<LayoutResult[]>{
  const directory=fs.mkdtempSync(path.join(tmpdir(),"atsumi-global-layout-input-"));
  try {
    const fixture=path.join(directory,"input.json");
    const artifacts=env.ATSUMI_LAYOUT_ARTIFACTS;
    fs.writeFileSync(fixture,JSON.stringify({scenes,css,width,height,browser:layoutBrowser,artifacts:artifacts&&group?path.join(artifacts,group):artifacts,captureAll:env.ATSUMI_LAYOUT_CAPTURE_ALL==="1"}));
    const stdout=await new Promise<string>((resolve,reject)=>execFile(execPath,[path.join(cwd(),"tools","layout-browser.mjs"),fixture],{timeout:150000,maxBuffer:8*1024**2,encoding:"utf8",windowsHide:true},(error,output,stderr)=>error?reject(new Error(stderr||error.message)):resolve(output)));
    return JSON.parse(stdout) as LayoutResult[];
  } finally {
    const target=path.relative(fs.realpathSync(tmpdir()),fs.realpathSync(directory));
    if(!target.startsWith("atsumi-global-layout-input-")||target.includes("..")||path.isAbsolute(target))throw new Error("Unexpected fixture cleanup target");
    fs.rmSync(directory,{recursive:true,force:true,maxRetries:10,retryDelay:100});
  }
}
