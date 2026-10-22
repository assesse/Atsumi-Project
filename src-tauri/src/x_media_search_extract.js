// One bounded read-only projection of the rendered public search document,
// including existing below-fold results without scrolling or loading more.
// No hidden application state, script execution, requests, clicks or scrolling.
(() => {
  'use strict';
  const request = __ATSUMI_MEDIA_SEARCH__;
  const result = {nonce:request.nonce,state:'unknown',candidates:[]};
  const tidy = (value,max=512) => {
    const clean=String(value||'').slice(0,32768).replace(/[\u0000-\u001f\u007f]/g,' ').replace(/\s+/g,' ').trim();
    let result='',units=0;
    for(const char of clean){if(units+char.length>max)break;result+=char;units+=char.length;}
    return result;
  };
  const link = value => {
    if(typeof value!=='string'||!value||value.length>2048||/[\s\x00-\x1f\x7f\\]/.test(value))return null;
    try {const url=new URL(value,location.href);if(!/^https?:$/.test(url.protocol)||url.username||url.password||!url.hostname)return null;
      return url.href.length<=2048?url.href:null;} catch {return null;}
  };
  const title=tidy(document.title,256);
  const path=location.pathname;
  const host=location.hostname;
  const expected=request.provider==='lens'
    ?location.protocol==='https:'&&((host==='lens.google.com'&&['/v3/upload','/upload','/search'].includes(path))||(host==='www.google.com'&&(path==='/search'||path.startsWith('/sorry/'))))
    :request.provider==='saucenao'&&location.protocol==='https:'&&host==='saucenao.com'&&['/','/search.php'].includes(path);
  if(!expected)return result;
  // One iterative projection, not 250 recursive subtree scans. These are work
  // limits, not just output limits: every visited node/CSS/rect is read once,
  // hidden subtrees are skipped, and a wide/deep tree cannot enqueue unbounded
  // siblings. Browser layout itself is not preemptible; the native caller also
  // bounds its wait. An incomplete projection never means "no matches".
  const started=performance.now(),MAX_NODES=6000,MAX_DEPTH=96,MAX_TEXT=32768;
  let visited=0,limited=false,plain='',challengeFrame=false;
  const outline=[],textBlocks=[],headings=[],mains=[],sauceCards=[],sauceContainers=new Set();
  const sauceEnabled=request.provider==='saucenao'&&(request.saucenaoVerified===true||request.probe===true);
  const root=document.documentElement;
  const frames=root?[{node:root,depth:0,entered:false,parent:null,anchor:null,main:null}]:[];
  const textOf=(record,max)=>tidy(plain.slice(record.start,record.end??plain.length),max);
  while(frames.length){
    if(performance.now()-started>150){limited=true;break;}
    const frame=frames[frames.length-1],node=frame.node;
    if(!frame.entered){
      if(++visited>MAX_NODES||frame.depth>MAX_DEPTH){limited=true;break;}
      frame.entered=true;
      if(node.nodeType===3){
        if(frame.parent?.visible){
          const value=node.nodeValue||'',remaining=MAX_TEXT-plain.length;
          if(value.length+1>remaining){limited=true;plain+=value.slice(0,Math.max(0,remaining));break;}
          plain+=value+' ';
        }
        frames.pop();continue;
      }
      if(!(node instanceof Element)||!node.isConnected
        ||/^(?:HEAD|SCRIPT|STYLE|TEMPLATE|NOSCRIPT)$/.test(node.tagName)
        ||node.hasAttribute('hidden')||node.hasAttribute('inert')||node.getAttribute('aria-hidden')==='true'){
        frames.pop();continue;
      }
      const style=getComputedStyle(node);
      if(style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse'||Number(style.opacity)===0){frames.pop();continue;}
      const rect=node.getBoundingClientRect();
      // Existing below-fold result cards are rendered, not hidden data. A
      // viewport intersection would omit ordinary results below AI overviews.
      frame.visible=Number.isFinite(rect.width)&&Number.isFinite(rect.height)&&rect.width>0&&rect.height>0;
      frame.start=plain.length;
      if(node.tagName==='A')frame.anchor={node,visible:frame.visible,url:link(node.getAttribute('href'))};
      if(node.tagName==='MAIN'||node.getAttribute('role')==='main'){
        frame.main=node;if(frame.visible)mains.push(node);
      }
      const role=node.getAttribute('role');
      if(sauceEnabled){
        // The card/table/column hierarchy was observed in native fixture
        // 1792619714359. Keep each card's frames isolated during this same walk.
        const has=name=>node.classList.contains(name),parent=frame.parent;
        if(has('result')){
          if(frame.sauce)frame.sauce.invalid=true;
          frame.sauce=null;frame.sauceTable=null;frame.sauceCell=null;frame.sauceContent=null;frame.sauceColumn=null;
          if(frame.visible&&node.id!=='result-hidden-notification'&&node.tagName==='DIV'
            &&parent?.visible&&parent.node.id==='middle'&&parent.parent?.visible&&parent.parent.node.id==='mainarea'){
            if(sauceCards.length>=32){limited=true;break;}
            const card={invalid:false,tables:[],cells:[],contents:[],titles:[],similarities:[],columns:[]};
            frame.sauce=card;sauceCards.push(card);sauceContainers.add(parent.node);
          }
        }
        const card=frame.sauce;
        if(card&&frame.visible){
          const collect=(list,value,cap)=>{if(list.length>=cap)card.invalid=true;else list.push(value);};
          if(node.tagName==='TABLE'&&has('resulttable')){frame.sauceTable=node;collect(card.tables,node,2);}
          if(node.tagName==='TD'&&has('resulttablecontent')&&frame.sauceTable){frame.sauceCell=node;collect(card.cells,node,2);}
          if(node.tagName==='DIV'&&has('resultcontent')&&parent?.node===frame.sauceCell){frame.sauceContent=node;collect(card.contents,node,2);}
          if(node.tagName==='DIV'&&has('resultsimilarityinfo')&&parent?.node.classList.contains('resultmatchinfo')&&parent.parent?.node===frame.sauceCell)collect(card.similarities,frame,2);
          if(node.tagName==='DIV'&&has('resulttitle')&&parent?.node===frame.sauceContent)collect(card.titles,frame,2);
          if(node.tagName==='DIV'&&has('resultcontentcolumn')&&parent?.node===frame.sauceContent){frame.sauceColumn={frame,links:[]};collect(card.columns,frame.sauceColumn,8);}
          if(node.tagName==='A'&&has('linkify')&&frame.sauceColumn&&parent===frame.sauceColumn.frame){
            frame.creatorMarked=/(?:^|\s)(?:Member|Author):\s*$/.test(tidy(plain.slice(frame.sauceColumn.frame.start,frame.start),2048));
            collect(frame.sauceColumn.links,frame,16);
          }
        }
      }
      if(frame.visible){
        if(node.tagName==='IFRAME'&&/recaptcha|turnstile/.test(node.getAttribute('src')||'')||node.id.startsWith('cf-chl'))challengeFrame=true;
        if(node.tagName==='H3'&&headings.length<128)headings.push(frame);
        if(request.probe===true&&outline.length<250&&(/^(?:H[1-4]|A|BUTTON|LABEL|P|SECTION|ARTICLE)$/.test(node.tagName)||role==='main'||role==='heading'))outline.push(frame);
      }
      frame.child=node.firstChild;
    }
    if(frame.child){
      const child=frame.child;frame.child=child.nextSibling;
      frames.push({node:child,depth:frame.depth+1,entered:false,parent:frame,anchor:frame.anchor,main:frame.main,
        sauce:frame.sauce,sauceTable:frame.sauceTable,sauceCell:frame.sauceCell,sauceContent:frame.sauceContent,sauceColumn:frame.sauceColumn});
    }else{
      frame.end=plain.length;
      // Extra probe-only structural evidence must not crowd semantic headings
      // and links out of their separate 250-item outline. No class is assumed.
      if(request.probe===true&&frame.visible&&textBlocks.length<64&&/^(?:DIV|SPAN)$/.test(node.tagName)){
        const text=textOf(frame,701);if(text&&text.length<=700)textBlocks.push(frame);
      }
      frames.pop();
    }
  }
  const body=tidy(plain,MAX_TEXT);
  if(request.probe===true) {
    // Dedicated synthetic-fixture runner only. Bound and omit script/hidden
    // content; keep the exact semantic visible shape to review parser changes.
    const observed=[],blocks=[];let evidenceBudget=256*1024,evidenceLimited=false;
    const evidenceUrl=url=>{
      if(!url)return null;
      const parsed=new URL(url);
      return /(^|\.)google\.com$/.test(parsed.hostname)?parsed.origin+parsed.pathname:url;
    };
    const describe=node=>({tag:node.tagName.toLowerCase(),id:tidy(node.id,80),class:tidy(node.getAttribute('class'),240),role:tidy(node.getAttribute('role'),40)});
    const evidenceItem=record=>{
      const node=record.node;
      const url=node.tagName==='A'?link(node.getAttribute('href')):null;
      const ancestors=[];
      for(let parent=record.parent,depth=0;parent&&depth<4;parent=parent.parent,depth++){
        if(parent.visible)ancestors.push(describe(parent.node));
      }
      const item={...describe(node),text:textOf(record,700),url:evidenceUrl(url),ancestors};
      if(node.tagName==='H3')item.context={
        main:record.main?{tag:record.main.tagName.toLowerCase(),id:tidy(record.main.id,80),role:tidy(record.main.getAttribute('role'),40)}:null,
        anchor:record.anchor?{url:evidenceUrl(record.anchor.url),rendered:record.anchor.visible}:null,
      };
      return item;
    };
    const append=(record,list)=>{
      const item=evidenceItem(record);
      const bytes=new TextEncoder().encode(JSON.stringify(item)).byteLength;
      if(bytes>evidenceBudget){evidenceLimited=true;return false;}
      evidenceBudget-=bytes;list.push(item);return true;
    };
    for(const record of outline)if(!append(record,observed))break;
    for(const record of textBlocks)if(!append(record,blocks))break;
    result.probe={title,body:tidy(body,24000),outline:observed,textBlocks:blocks,limited,evidenceLimited};
  }
  // Terminal provider/security screens are not empty successful searches.
  if(path.startsWith('/sorry/')||/^Just a moment/i.test(title)||/unusual traffic|verify (?:you are|that you are) human|checking your browser|사람인지 확인|비정상적인 트래픽/i.test(body)
    ||challengeFrame) {result.state='challenge';return result;}
  if(/before you continue to google|Google로 이동하기 전에/i.test(body)){result.state='consent';return result;}
  if(/daily search limit|search limit exceeded|too many searches|일일 검색 한도/i.test(body)){result.state='quota';return result;}
  if(limited)return result;
  if(document.readyState!=='complete'||!body){result.state='pending';return result;}
  // Layout gates intentionally stay false until actual public rendered fixture
  // evidence exists. A fabricated fixture cannot enable production completion.
  if(request.provider==='lens'&&(request.lensVerified===true||request.probe===true)) {
    if(mains.length!==1)return result;
    const main=mains[0];
    const seen=new Set();
    for(const heading of headings) {
      if(result.candidates.length>=10)break;
      const anchor=heading.anchor;
      if(heading.main!==main||!anchor?.visible||!main.contains(anchor.node))continue;
      const url=anchor.url;if(!url||seen.has(url))continue;
      const parsed=new URL(url);
      // This is the actually rendered Lens result link, not a decoded target.
      // Keep Google's opaque /goto URL exactly as observed for a later explicit
      // user open; never infer an artist or invent a destination from its token.
      if(/(^|\.)google\.com$/.test(parsed.hostname)
        &&!(parsed.origin==='https://www.google.com'&&parsed.pathname==='/goto'
          &&parsed.searchParams.getAll('url').length===1&&parsed.searchParams.get('url')))continue;
      const text=textOf(heading,512);if(!text)continue;
      seen.add(url);result.candidates.push({title:text,creator:null,similarity:null,links:[{label:parsed.hostname,url}]});
    }
    if(result.candidates.length)result.state='complete';
  }
  if(sauceEnabled&&sauceContainers.size===1){
    const seen=new Set();
    for(const card of sauceCards){
      if(result.candidates.length>=10)break;
      if(card.invalid||card.tables.length!==1||card.cells.length!==1||card.contents.length!==1||card.titles.length!==1||card.similarities.length!==1||!card.columns.length)continue;
      const text=textOf(card.titles[0],512),score=textOf(card.similarities[0],32);
      if(!text||!/^\d{1,3}(?:\.\d+)?%$/.test(score))continue;
      const similarity=Number(score.slice(0,-1));if(!Number.isFinite(similarity)||similarity>100)continue;
      const links=[],creators=new Set(),urls=new Set();
      for(const column of card.columns)for(const anchor of column.links){
        const url=anchor.anchor?.url;if(!url)continue;
        const parsed=new URL(url);if(/(^|\.)saucenao\.com$/.test(parsed.hostname))continue;
        const label=textOf(anchor,256);if(!label)continue;
        if(anchor.creatorMarked)creators.add(label);
        if(!urls.has(url)){urls.add(url);if(links.length<3)links.push({label,url});}
      }
      if(!links.length||seen.has(links[0].url))continue;
      seen.add(links[0].url);result.candidates.push({title:text,creator:creators.size===1?[...creators][0]:null,similarity,links});
    }
    if(result.candidates.length)result.state='complete';
  }
  return result;
})()
