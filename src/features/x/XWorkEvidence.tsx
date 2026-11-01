import { useEffect, useState } from "react";
import type { XMediaSearchApi, XWorkEvidence as Evidence, XWorkInfo } from "./xMediaSearchApi";
import { isXCreatorPostLink } from "./xMediaSearchApi";
import { SourceText } from "./XPageEvidence";

export function XWorkEvidence({api,jobId,sourceKey,phrase,busy}:{api:XMediaSearchApi;jobId:string;sourceKey:string;phrase:string;busy:boolean}){
  return <WorkEvidence key={`${jobId}:${sourceKey}`} api={api} jobId={jobId} phrase={phrase} busy={busy}/>;
}
function WorkEvidence({api,jobId,phrase,busy}:{api:XMediaSearchApi;jobId:string;phrase:string;busy:boolean}){
  const [data,setData]=useState<Evidence|null>(null),[error,setError]=useState(""),[opening,setOpening]=useState(false);
  useEffect(()=>{
    if(!api.workEvidence)return;
    let live=true;
    void api.workEvidence(jobId).then(value=>{if(live)setData(value);}).catch(()=>{if(live)setError("작품·판매 정보를 확인하지 못했습니다.");});
    return()=>{live=false;};
  },[api,jobId]);
  if(!api.workEvidence)return null;
  const open=async(index:number,origin=false)=>{
    if(!data||opening||busy||!api.openWorkEvidence)return;
    setOpening(true);setError("");try{if(origin)await api.openWorkEvidence(jobId,data.reportId,index,true);else await api.openWorkEvidence(jobId,data.reportId,index);}catch{setError("출처 페이지를 열지 못했습니다.");}finally{setOpening(false);}
  };
  const groups=new Map<string,{work:XWorkInfo;index:number}[]>();
  data?.works.forEach((work,index)=>{const key=work.context?`group-${work.context.group}`:`legacy-${index}`;const group=groups.get(key)||[];group.push({work,index});groups.set(key,group);});
  const groupEntries=[...groups];
  const primaryGroups=groupEntries.filter(([,entries],i)=>i===0||entries.some(({work})=>["source_link","supported"].includes(work.context?.status||""))).slice(0,3);
  const otherGroups=groupEntries.filter(([key])=>!primaryGroups.some(([primary])=>primary===key));
  const source=(work:XWorkInfo,index:number)=>{
    const context=work.context;
    const linked=!context||!["unverified","conflict"].includes(context.association);
    const facts=<dl>{[...work.codes,...work.sale].map((field,i)=><div key={i}><dt title={field.source}>{field.label}</dt><dd><SourceText text={field.value} phrase={phrase}/></dd></div>)}</dl>;
    return <div className="x-work-source" key={index}>
      <h4><button type="button" className="x-media-search-title" disabled={busy||opening} title={work.url} onClick={()=>void open(index)}><SourceText text={work.title||"출처에 표시된 작품 정보"} phrase={phrase}/> <small>{new URL(work.url).hostname}</small></button></h4>
      {context?<p className={`x-work-status${context.status==="conflict"?" x-work-conflict":""}`}>{context.kind==="upload"?"업로드 제목 · ":context.kind==="product"?"상품 정보 · ":""}{{candidate:"검색 후보",supported:"두 검색 서비스에서 일치",source_link:"게시자 연결 출처",conflict:"정보 충돌 · 연결 보류",page:"출처 페이지 정보"}[context.status]}</p>:null}
      {context?.pageTitle&&context.pageTitle!==work.title?<p className="x-media-search-muted">페이지 제목: <SourceText text={context.pageTitle} phrase={phrase}/></p>:null}
      {context?.pageStatus==="unavailable"?<p className="x-media-search-muted">페이지 읽기 실패 · 검색 후보 유지</p>:context?.pageStatus==="not_read"?<p className="x-media-search-muted">상세 페이지 미확인</p>:null}
      {work.codes.length===0&&work.sale.length===0?<p className="x-media-search-muted" title={work.titleEvidence?.source}>품번·판매 정보 미확인</p>:null}
      {work.codes.length>0&&work.sale.length===0?<p className="x-media-search-muted">판매 정보 미확인</p>:work.codes.length===0&&work.sale.length>0?<p className="x-media-search-muted">품번 미확인</p>:null}
      {work.rootUrl!==work.url&&isXCreatorPostLink(work.rootUrl)&&new URL(work.rootUrl).hostname==="x.com"?<button type="button" className="x-work-evidence-origin" title={work.rootUrl} disabled={busy||opening} onClick={()=>void open(index,true)}>{work.origin}</button>:<span className="x-work-evidence-origin" title={work.rootUrl}>{work.origin}{work.rootUrl!==work.url?" · 게시된 연결 페이지":null}</span>}
      {linked?facts:work.codes.length||work.sale.length?<details className="x-work-unlinked"><summary>연결하지 않은 페이지 정보</summary>{facts}</details>:null}
      {context?.reasons.map(reason=><p key={reason} className="x-media-search-muted">{reason}</p>)}
      {context?.claims.length?<details className="x-work-claims"><summary>검색 근거 {context.claims.length}개</summary>{context.claims.map((claim,i)=><div key={i}><b>{claim.provider==="saucenao"?"SauceNAO":"Google Lens"} {claim.position}</b>{claim.similarity!==null?<span> · 유사도 {claim.similarity}%</span>:null}<p><SourceText text={claim.title} phrase={phrase}/></p>{claim.creator?<p>검색 결과의 작성자 표기: {claim.creator}</p>:null}{claim.metadata.map((field,i)=><p key={i}>{field.label}: <SourceText text={field.value} phrase={phrase}/></p>)}</div>)}</details>:null}
      {work.ambiguous?<p className="x-media-search-muted">여러 항목이 있어 작품을 확정하지 않았습니다.</p>:null}
    </div>;
  };
  const renderGroups=(entries:typeof groupEntries)=><ul>{entries.map(([key,items])=><li key={key} className="x-work-group">{items.length>1?<p className="x-work-group-label">같은 후보의 출처 {items.length}개 · 정보는 출처별로 표시</p>:null}{items.map(({work,index})=>source(work,index))}</li>)}</ul>;
  return <section className="x-work-evidence" aria-label="작품·판매 정보" aria-busy={!data&&!error}>
    <h3>작품·판매 정보</h3>
    {!data&&!error?<p role="status" className="x-media-search-inline-status"><i className="x-media-search-spinner" aria-hidden="true"/>출처에 적힌 작품 코드와 판매 정보 확인 중…</p>:null}
    {data?<>
      {data.works.length?<>{renderGroups(primaryGroups)}{otherGroups.length?<details className="x-work-alternatives"><summary>다른 후보·미연결 페이지 {otherGroups.length}개</summary>{renderGroups(otherGroups)}</details>:null}</>:<p>{data.examined?"확인한 페이지에서 명시된 작품 코드를 찾지 못했습니다.":"출처 페이지의 작품 정보를 읽지 못했습니다."}</p>}
      <p className="x-media-search-muted">{data.examined}개 페이지 확인{data.unavailable?` · ${data.unavailable}개 읽기 실패`:""}{data.limited?" · 일부 정보만 확인됨":""}</p>
      {data.sourceStatus==="unavailable"?<p className="x-media-search-muted">게시물·타래의 등록 링크를 확인하지 못했습니다.</p>:data.sourceStatus==="partial"?<p className="x-media-search-muted">게시물·타래의 등록 링크는 일부만 확인했습니다.</p>:null}
      {data.works.length?<p className="x-media-search-muted">출처에 적힌 정보입니다. 검색 유사도만으로 원본 작품을 확정하지 않습니다.</p>:null}
    </>:null}
    {error?<p role="status">{error}</p>:null}
  </section>;
}
