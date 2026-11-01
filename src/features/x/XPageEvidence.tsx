import { Fragment, useEffect, useRef, useState } from "react";
import type { XMediaSearchApi, XPageEvidence as Evidence, XSearchProvider } from "./xMediaSearchApi";

export function SourceText({text,phrase=""}:{text:string;phrase?:string}) {
  const needle=phrase.trim();if(!needle)return <>{text}</>;
  // ASCII case folding preserves offsets for literal Unicode phrases.
  const fold=(value:string)=>value.replace(/[A-Z]/g,char=>char.toLowerCase());
  const haystack=fold(text),wanted=fold(needle),parts=[];let offset=0,found=haystack.indexOf(wanted);
  while(found>=0&&parts.length<128){parts.push(<Fragment key={found}>{text.slice(offset,found)}<mark>{text.slice(found,found+needle.length)}</mark></Fragment>);offset=found+needle.length;found=haystack.indexOf(wanted,offset);}
  parts.push(text.slice(offset));return <>{parts}</>;
}

type Props={api:XMediaSearchApi;jobId:string;candidateIndex:number;provider?:XSearchProvider;links:{label:string;url:string}[];phrase:string};
export function XPageEvidence(props:Props) {
  // A refreshed result is a new disclosure. It must not inherit an old source
  // selection or start another page read without a new user expansion.
  const key=JSON.stringify([props.jobId,props.provider,props.candidateIndex,props.links.map(link=>link.url)]);
  return <EvidenceDetails key={key} {...props}/>;
}
function EvidenceDetails({api,jobId,candidateIndex,provider,links,phrase}:Props) {
  const [open,setOpen]=useState(false),[selected,setSelected]=useState(0),[revision,setRevision]=useState(0);
  const [data,setData]=useState<Evidence|null>(null),[pending,setPending]=useState(false),[error,setError]=useState("");
  const [loadedFor,setLoadedFor]=useState("");
  const cache=useRef(new Map<string,Evidence>()),identity=`${jobId}:${provider??"default"}:${candidateIndex}`;
  const target=links[selected]?.url??"";
  useEffect(()=>{
    let live=true;setData(null);setLoadedFor("");setError("");setPending(false);
    if(!open||!target)return;
    const saved=cache.current.get(target);if(saved){setData(saved);setLoadedFor(identity);return;}
    if(!api.pageEvidence){setError("이 검색에서 페이지 정보를 확인할 수 없습니다.");return;}
    setPending(true);
    void api.pageEvidence(jobId,candidateIndex,selected,provider).then(value=>{
      if(!live)return;
      if(value.requestedUrl!==target)throw new Error("source changed");
      cache.current.set(target,value);setData(value);setLoadedFor(identity);
    }).catch(()=>{if(live)setError("페이지 정보를 읽지 못했습니다. 원본 페이지에서 확인해 주세요.");})
      .finally(()=>{if(live)setPending(false);});
    return()=>{live=false;};
  },[api,jobId,candidateIndex,provider,open,selected,target,revision]);
  return <details className="x-page-evidence" onToggle={event=>setOpen(event.currentTarget.open)}>
    <summary>페이지 정보</summary>
    {open?<div className="x-page-evidence-body">
      {links.length>1?<div className="x-page-evidence-sources" aria-label="정보를 확인할 출처">{links.map((link,index)=><button key={index} type="button" aria-pressed={selected===index} disabled={pending} title={link.url} onClick={()=>setSelected(index)}>{index+1} · {new URL(link.url).hostname}</button>)}</div>:null}
      {pending?<p role="status">페이지 확인 중…</p>:null}
      {error?<div role="status"><p>{error}</p><button type="button" onClick={()=>{cache.current.delete(target);setRevision(value=>value+1);}}>다시 확인</button></div>:null}
      {data&&loadedFor===identity&&data.requestedUrl===target?<>
        <div className="x-page-evidence-origin"><span title={data.url}>{new URL(data.url).hostname}</span><time dateTime={data.observedAt}>{new Date(data.observedAt).toLocaleTimeString("ko-KR")}</time></div>
        {data.fields.length?<dl>{data.fields.map((field,index)=><div key={index}><dt title={field.source}>{field.label}</dt><dd><SourceText text={field.value} phrase={phrase}/></dd></div>)}</dl>:<p>읽어 낸 정보 없음</p>}
        {data.ambiguous?<p className="x-page-evidence-note">여러 항목의 정보가 있어 해당 페이지의 항목을 확정하지 않았습니다.</p>:null}
        {data.limited?<p className="x-page-evidence-note">긴 정보는 일부만 표시됩니다.</p>:null}
      </>:null}
    </div>:null}
  </details>;
}
