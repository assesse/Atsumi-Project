import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FluentIcon } from "../../components/FluentIcon";
export function BrowserLauncher(){const [busy,setBusy]=useState(false),[error,setError]=useState<string|null>(null);return <><button type="button" className="nav-item" style={{width:"100%"}} aria-label="브라우저 열기" title={error??"브라우저 · VPN"} disabled={busy} onClick={()=>{setBusy(true);setError(null);void invoke("general_browser_open").catch(e=>setError(String(e))).finally(()=>setBusy(false));}}><FluentIcon glyph="\uE774"/><span className="nav-label">브라우저</span></button>{error&&<span role="alert" style={{fontSize:14,padding:8}}>{error}</span>}</>;}
