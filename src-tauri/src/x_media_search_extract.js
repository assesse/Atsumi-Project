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
  const thumbnail = value => {
    if(request.provider==='lens'&&typeof value==='string'&&value.length<=128*1024&&/^data:image\/(?:jpeg|png|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(value))return value;
    const url=link(value);if(!url)return null;
    const parsed=new URL(url);if(parsed.protocol!=='https:'||parsed.port||parsed.hash)return null;
    if(request.provider==='saucenao')return /^img[1-9]\.saucenao\.com$/.test(parsed.hostname)
      &&(parsed.pathname.startsWith('/res/')||/^\/artstation\/[0-9]{1,20}\/[0-9]{1,20}_[0-9]{1,20}\.jpg$/.test(parsed.pathname))?url:null;
    return /^encrypted-tbn[0-9]\.gstatic\.com$/.test(parsed.hostname)&&parsed.pathname==='/images'?url:null;
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
  const outline=[],textBlocks=[],probeImages=[],headings=[],mains=[],mainFrames=[],emptyColumns=[],anchors=[],navigationControls=[],sauceCards=[],sauceLowNotices=[],sauceContainers=new Set();
  const sauceEnabled=request.provider==='saucenao'&&(request.saucenaoVerified===true||request.probe===true);
  const root=document.documentElement;
  const frames=root?[{node:root,depth:0,entered:false,parent:null,anchor:null,main:null}]:[];
  const textOf=(record,max)=>tidy(plain.slice(record.start,record.end??plain.length),max);
  while(frames.length){
    if(performance.now()-started>150){limited=true;break;}
    const frame=frames[frames.length-1],node=frame.node;
    if(!frame.entered){
      if(++visited>MAX_NODES||frame.depth>MAX_DEPTH){limited=true;break;}
      frame.ordinal=visited;
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
      if(node.tagName==='NAV'||node.getAttribute('role')==='navigation')frame.navigation=node;
      if(node.tagName==='A'){
        frame.anchor={node,visible:frame.visible,url:link(node.getAttribute('href')),start:frame.start,end:null,images:[],previewImages:[],headingCount:0,main:frame.main,navigation:frame.navigation};
        anchors.push(frame.anchor);
      }
      if(node.tagName==='IMG'&&frame.visible&&frame.anchor){
        const source=thumbnail(node.getAttribute('src'));
        if(source&&frame.anchor.images.length<2)frame.anchor.images.push(source);
        if(request.provider==='lens'&&rect.width>=48&&rect.height>=48&&frame.anchor.previewImages.length<2)frame.anchor.previewImages.push(source);
      }
      if(request.provider==='lens'&&frame.visible){
        const p=frame.parent;
        const ancestors=[];for(let parent=p,depth=0;parent&&depth<8;parent=parent.parent,depth++)ancestors.push(parent);
        const cardIndex=ancestors.findIndex(parent=>parent.node.tagName==='DIV'&&parent.node.classList.contains('N54PNb'));
        const card=cardIndex<0?null:ancestors[cardIndex];
        // Actual neutral result DOM: the full photo and title link are separate
        // columns of the SAME outer N54PNb card. Nested title N54PNb nodes and
        // favicon/AI/query images cannot establish this ownership relation.
        if(node.tagName==='A'&&node.classList.contains('LBcIee')&&p?.node.classList.contains('kb0PBd')&&card){
          frame.anchor.visualOwner=card;
          const owned=card.visualAnchors??(card.visualAnchors=[]);if(owned.length<2)owned.push(frame.anchor);
        }
        if(node.tagName==='IMG'&&!frame.anchor&&rect.width>=48&&rect.height>=48
          &&p?.node.classList.contains('pb66sc')&&card){
          // The observed T3Fozb display-contents wrapper has a zero rect. Keep
          // frame identity while allowing such wrappers between known columns.
          const before=ancestors.slice(1,cardIndex),button=before.findIndex(parent=>parent.node.classList.contains('DeNS1c')&&parent.node.getAttribute('role')==='button');
          const picture=before.findIndex(parent=>parent.node.classList.contains('R8rd3e')),column=before.findIndex(parent=>parent.node.classList.contains('kb0PBd')&&parent.node.classList.contains('pAEXpf'));
          if(button>=0&&picture>button&&column>picture){const owned=card.visualImages??(card.visualImages=[]);if(owned.length<2)owned.push(thumbnail(node.getAttribute('src')));}
        }
      }
      if(node.tagName==='MAIN'||node.getAttribute('role')==='main'){
        frame.main=node;if(frame.visible){mains.push(node);mainFrames.push(frame);}
      }
      // Observed Google exact-zero document: center_col can be a plain DIV.
      // Retain this frame for explicit empty notices only, never for cards.
      if(request.provider==='lens'&&frame.visible&&node.id==='center_col')emptyColumns.push(frame);
      const role=node.getAttribute('role');
      if(request.provider==='lens'&&frame.navigation&&(node.tagName==='A'||role==='link'||role==='tab'
        ||node.getAttribute('aria-current')==='page'||node.getAttribute('aria-selected')==='true'))navigationControls.push(frame);
      if(sauceEnabled){
        // The card/table/column hierarchy was observed in native fixture
        // 1792619714359. Keep each card's frames isolated during this same walk.
        const has=name=>node.classList.contains(name),parent=frame.parent;
        if(has('result')){
          if(frame.sauce)frame.sauce.invalid=true;
          frame.sauce=null;frame.sauceTable=null;frame.sauceCell=null;frame.sauceContent=null;frame.sauceColumn=null;frame.sauceMisc=null;
          if(frame.visible&&node.id==='result-hidden-notification'&&node.tagName==='DIV'
            &&parent?.visible&&parent.node.id==='middle'&&parent.parent?.visible&&parent.parent.node.id==='mainarea'){
            if(sauceLowNotices.length>=2){limited=true;break;}
            sauceLowNotices.push(frame);
          }
          if(frame.visible&&node.id!=='result-hidden-notification'&&node.tagName==='DIV'
            &&parent?.visible&&parent.node.id==='middle'&&parent.parent?.visible&&parent.parent.node.id==='mainarea'){
            if(sauceCards.length>=32){limited=true;break;}
            const card={invalid:false,frame,tables:[],cells:[],contents:[],titles:[],similarities:[],columns:[],misc:[],images:[]};
            frame.sauce=card;sauceCards.push(card);sauceContainers.add(parent.node);
          }
        }
        const card=frame.sauce;
        if(card&&frame.visible){
          const collect=(list,value,cap)=>{if(list.length>=cap)card.invalid=true;else list.push(value);};
          if(node.tagName==='TABLE'&&has('resulttable')){frame.sauceTable=node;collect(card.tables,node,2);}
          if(node.tagName==='IMG'&&frame.sauceTable&&parent?.node.tagName==='A'&&parent.parent?.node.classList.contains('resultimage')
            &&parent.parent.parent?.node.tagName==='TD'&&parent.parent.parent.node.classList.contains('resulttableimage')){
            const source=thumbnail(node.getAttribute('src'));if(source)collect(card.images,source,2);
          }
          if(node.tagName==='TD'&&has('resulttablecontent')&&frame.sauceTable){frame.sauceCell=node;collect(card.cells,node,2);}
          if(node.tagName==='DIV'&&has('resultcontent')&&parent?.node===frame.sauceCell){frame.sauceContent=node;collect(card.contents,node,2);}
          if(node.tagName==='DIV'&&has('resultsimilarityinfo')&&parent?.node.classList.contains('resultmatchinfo')&&parent.parent?.node===frame.sauceCell)collect(card.similarities,frame,2);
          if(node.tagName==='DIV'&&has('resultmiscinfo')&&parent?.node.classList.contains('resultmatchinfo')&&parent.parent?.node===frame.sauceCell){
            frame.sauceMisc={frame,links:[]};collect(card.misc,frame.sauceMisc,2);
          }
          if(node.tagName==='A'&&frame.sauceMisc&&parent===frame.sauceMisc.frame){
            frame.sourceIcon=false;collect(frame.sauceMisc.links,frame,8);
          }
          if(node.tagName==='IMG'&&parent?.node.tagName==='A'&&parent.parent===frame.sauceMisc?.frame
            &&parent.node.childElementCount===1){
            try {const icon=new URL(node.getAttribute('src'),location.href);
              parent.sourceIcon=icon.origin===location.origin&&/^\/images\/static\/siteicons\/[A-Za-z0-9_-]+\.ico$/.test(icon.pathname)&&!icon.search&&!icon.hash;
            } catch { /* unknown icon shape */ }
          }
          if(node.tagName==='DIV'&&has('resulttitle')&&parent?.node===frame.sauceContent)collect(card.titles,frame,2);
          if(node.tagName==='DIV'&&has('resultcontentcolumn')&&parent?.node===frame.sauceContent){frame.sauceColumn={frame,links:[],breaks:[]};collect(card.columns,frame.sauceColumn,8);}
          if(node.tagName==='A'&&has('linkify')&&frame.sauceColumn&&parent===frame.sauceColumn.frame){
            frame.creatorMarked=/(?:^|\s)(?:Member|Author):\s*$/.test(tidy(plain.slice(frame.sauceColumn.frame.start,frame.start),2048));
            collect(frame.sauceColumn.links,frame,16);
          }
        }
        // Preserve only actual rendered field separators, not newlines in HTML
        // source text. A normal BR can have zero width; its visible parent and
        // the CSS/hidden checks above establish whether it separates fields.
        if(card&&node.tagName==='BR'&&frame.sauceColumn&&parent===frame.sauceColumn.frame&&parent.visible){
          if(frame.sauceColumn.breaks.length>=32)card.invalid=true;
          else frame.sauceColumn.breaks.push(plain.length);
        }
      }
      if(frame.visible){
        if(node.tagName==='IFRAME'&&/recaptcha|turnstile/.test(node.getAttribute('src')||'')||node.id.startsWith('cf-chl'))challengeFrame=true;
        // The traversal's MAX_NODES bounds this collection. Truncating only
        // headings would misclassify later rendered result titles as notices.
        // Actual Lens visual cards use DIV[role=heading], while ordinary web
        // results use H3. Keep the rendered heading's same-link ownership; a
        // generic DIV/text or AI citation is not a result title.
        if(node.tagName==='H3'||role==='heading'){
          if(frame.anchor)frame.anchor.headingCount++;
          if(node.tagName==='H3')headings.push(frame);
          else {
            const p=frame.parent;
            frame.visualHeading=!!frame.anchor&&node.tagName==='DIV'&&node.classList.contains('HePszc')
              &&p?.node.classList.contains('lTYpse')&&p.parent?.node.classList.contains('kb0PBd')
              &&p.parent.parent?.node.classList.contains('N54PNb')
              &&p.parent.parent.parent?.node===frame.anchor?.node&&frame.anchor.node.classList.contains('LBcIee');
            if(frame.visualHeading){frame.anchor.visualHeading=true;headings.push(frame);}
          }
        }
        if(request.probe===true&&node.tagName==='IMG'&&probeImages.length<32){frame.imageSize={width:rect.width,height:rect.height};probeImages.push(frame);}
        if(request.probe===true&&outline.length<250&&(/^(?:H[1-4]|A|BUTTON|LABEL|P|SECTION|ARTICLE)$/.test(node.tagName)||role==='main'||role==='heading'))outline.push(frame);
      }
      frame.child=node.firstChild;
    }
    if(frame.child){
      const child=frame.child;frame.child=child.nextSibling;
      frames.push({node:child,depth:frame.depth+1,entered:false,parent:frame,anchor:frame.anchor,main:frame.main,navigation:frame.navigation,
        sauce:frame.sauce,sauceTable:frame.sauceTable,sauceCell:frame.sauceCell,sauceContent:frame.sauceContent,sauceColumn:frame.sauceColumn,sauceMisc:frame.sauceMisc});
    }else{
      frame.end=plain.length;
      if(frame.anchor?.node===node)frame.anchor.end=frame.end;
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
    const observed=[],blocks=[],images=[];let evidenceBudget=256*1024,evidenceLimited=false;
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
      const ownership=request.provider==='lens'&&node.tagName==='A'&&node.classList.contains('LBcIee');
      for(let parent=record.parent,depth=0;parent&&depth<(ownership?8:4);parent=parent.parent,depth++){
        if(parent.visible||ownership)ancestors.push({...describe(parent.node),ordinal:parent.ordinal,depth,rendered:!!parent.visible});
      }
      const item={...describe(node),ordinal:record.ordinal,text:textOf(record,700),url:evidenceUrl(url),ancestors};
      if(node.tagName==='H3'||node.getAttribute('role')==='heading')item.context={
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
    for(const record of probeImages){
      const node=record.node,raw=node.getAttribute('src')||'',admitted=thumbnail(raw)!==null;
      let source={kind:'other',chars:raw.length};
      if(raw.startsWith('data:')){const header=raw.slice(0,128),end=header.indexOf(';');source={kind:'data',chars:raw.length,mime:tidy(header.slice(5,end>5?end:64),64)};}
      else if(raw.length<=2048){try{const url=new URL(raw,location.href);source={kind:'url',host:url.hostname,path:url.pathname,chars:raw.length};}catch{}}
      const ancestors=[];for(let parent=record.parent,depth=0;parent&&depth<8;parent=parent.parent,depth++)ancestors.push({...describe(parent.node),ordinal:parent.ordinal,depth,rendered:!!parent.visible});
      const item={...describe(node),ordinal:record.ordinal,source,admitted,size:record.imageSize,anchor:record.anchor?{...describe(record.anchor.node),url:evidenceUrl(record.anchor.url)}:null,ancestors};
      const bytes=new TextEncoder().encode(JSON.stringify(item)).byteLength;
      if(bytes>evidenceBudget){evidenceLimited=true;break;}evidenceBudget-=bytes;images.push(item);
    }
    result.probe={title,body:tidy(body,24000),outline:observed,textBlocks:blocks,images,limited,evidenceLimited};
  }
  // Actual challenge routes, document titles and rendered widgets always win,
  // even if the document also contains result-shaped markup.
  if(path.startsWith('/sorry/')||/^Just a moment/i.test(title)||challengeFrame){result.state='challenge';return result;}
  if(request.provider==='saucenao'&&title==='saucenao.com | 521: Web server is down'){result.state='unavailable';return result;}
  const noticeState=text=>{
    if(/unusual traffic|verify (?:you are|that you are) human|checking your browser|사람인지 확인|비정상적인 트래픽/i.test(text))return 'challenge';
    if(/before you continue to google|Google로 이동하기 전에/i.test(text))return 'consent';
    if(/daily search limit|search limit exceeded|too many searches|일일 검색 한도/i.test(text))return 'quota';
    return null;
  };
  if(limited||document.readyState!=='complete'||!body){
    result.state=noticeState(body)||(limited?'unknown':'pending');return result;
  }
  const resultRanges=[];
  const lensSource=url=>{
    if(!url)return false;
    const parsed=new URL(url);
    return !/(^|\.)google\.com$/.test(parsed.hostname)
      ||parsed.origin==='https://www.google.com'&&parsed.pathname==='/goto'&&parsed.searchParams.getAll('url').length===1&&!!parsed.searchParams.get('url');
  };
  const retainedLensRedirect=url=>{
    if(!url)return false;
    const parsed=new URL(url);
    return parsed.origin==='https://www.google.com'&&!parsed.hash
      &&['/goto','/url'].includes(parsed.pathname)&&!!parsed.search;
  };
  if(request.provider==='lens'&&request.preferExact===true){
    const modes={exact:[],visual:[],all:[]};
    for(const control of navigationControls){
      if(!control.navigation||control.main)continue;
      const text=textOf(control,120);if(!control.visible&&!text)continue;
      const mode=/^(?:정확히 일치하는 (?:결과|항목)|Exact matches)$/i.test(text)?'exact'
        :/^(?:시각적으로 일치하는 항목|Visual matches|Visually similar)$/i.test(text)?'visual'
        :/^(?:전체|All)$/i.test(text)?'all':null;
      if(mode)modes[mode].push({node:control.node,url:link(control.node.getAttribute('href'))});
    }
    const selected=[];
    const href=mode=>{
      const values=[...new Set(modes[mode].map(anchor=>anchor.url).filter(Boolean))];
      return values.length===1?values[0]:null;
    };
    for(const mode of Object.keys(modes))for(const anchor of modes[mode]){
      if(anchor.node.tagName==='A'&&!anchor.node.hasAttribute('href')||anchor.node.getAttribute('aria-current')==='page'||anchor.node.getAttribute('aria-current')==='true'
        ||anchor.node.getAttribute('aria-selected')==='true'||anchor.node.getAttribute('aria-disabled')==='true')selected.push(mode);
    }
    const active=[...new Set(selected)];
    result.lens={mode:active.length===1?active[0]:'unknown',exactUrl:href('exact'),visualUrl:href('visual'),allUrl:href('all')};
  }
  // Layout gates intentionally stay false until actual public rendered fixture
  // evidence exists. A fabricated fixture cannot enable production completion.
  if(request.provider==='lens'&&(request.lensVerified===true||request.probe===true)) {
    const main=mains.length===1?mains[0]:null;
    const seen=new Set();
    // The supplied exact-results appshot exposes whole linked image-list
    // cards, without a heading. Require one owned large image and the visible
    // source dimensions; generic links, source icons and AI citations do not
    // establish a card. This semantic variant still needs deferred live QA.
    if(result.lens?.mode==='exact')for(const anchor of anchors){
      if(!main||anchor.main!==main||!anchor.visible||anchor.previewImages.length!==1||!lensSource(anchor.url))continue;
      const full=textOf(anchor,2048);
      if(!/(?:^|\s)[\d,]{2,7}\s*[×xX]\s*[\d,]{2,7}(?:\s|$)/.test(full))continue;
      resultRanges.push([anchor.start,anchor.end]);
      if(result.candidates.length>=10||seen.has(anchor.url))continue;
      const candidate={title:tidy(full,512),creator:null,similarity:null,links:[{label:new URL(anchor.url).hostname,url:anchor.url}]};
      if(anchor.previewImages[0])candidate.thumbnailSource=anchor.previewImages[0];
      seen.add(anchor.url);result.candidates.push(candidate);
    }
    for(const heading of result.lens?.mode==='exact'?[]:headings) {
      const anchor=heading.anchor;
      if(!main||heading.main!==main||!anchor?.visible||!main.contains(anchor.node))continue;
      if(anchor.visualHeading&&anchor.headingCount!==1)continue;
      const url=anchor.url;if(!url)continue;
      const parsed=new URL(url);
      // This is the actually rendered Lens result link, not a decoded target.
      // Keep Google's opaque /goto URL exactly as observed for a later explicit
      // user open; never infer an artist or invent a destination from its token.
      if(/(^|\.)google\.com$/.test(parsed.hostname)
        &&!(parsed.origin==='https://www.google.com'&&parsed.pathname==='/goto'
          &&parsed.searchParams.getAll('url').length===1&&parsed.searchParams.get('url')))continue;
      const text=textOf(heading,512);if(!text)continue;
      resultRanges.push([anchor.start,anchor.end]);
      if(result.candidates.length>=10||seen.has(url))continue;
      const candidate={title:text,creator:null,similarity:null,links:[{label:parsed.hostname,url}]};
      if(anchor.visualHeading){
        const owner=anchor.visualOwner;
        if(owner?.visualAnchors?.length===1&&owner.visualAnchors[0]===anchor&&owner.visualImages?.length===1&&owner.visualImages[0])candidate.thumbnailSource=owner.visualImages[0];
      }else if(anchor.images.length===1)candidate.thumbnailSource=anchor.images[0];
      seen.add(url);result.candidates.push(candidate);
    }
    if(result.candidates.length)result.state='complete';
  }
  if(sauceEnabled&&sauceContainers.size===1){
    const seen=new Set();
    for(const card of sauceCards){
      if(card.invalid||card.tables.length!==1||card.cells.length!==1||card.contents.length!==1||card.titles.length>1||card.similarities.length!==1||!card.columns.length||card.misc.length>1)continue;
      const text=card.titles.length?textOf(card.titles[0],512):'',score=textOf(card.similarities[0],32);
      if(!/^\d{1,3}(?:\.\d+)?%$/.test(score))continue;
      const similarity=Number(score.slice(0,-1));if(!Number.isFinite(similarity)||similarity>100)continue;
      let links=[];
      const creators=[],explicitCreators=[],creatorLinks=[],sources=[],urls=new Set(),sourceLinks=new Map(),metadata=[];
      for(const column of card.columns)for(const anchor of column.links){
        const url=anchor.anchor?.url;if(!url)continue;
        const parsed=new URL(url);if(/(^|\.)saucenao\.com$/.test(parsed.hostname))continue;
        const label=textOf(anchor,256);if(!label)continue;
        sourceLinks.set(anchor,{label,url});
        if(anchor.creatorMarked)creators.push(label);
        if(!urls.has(url)){urls.add(url);if(links.length<3)links.push({label,url});}
      }
      // Creator/Source/Material are reported visible fields, but this exact DOM
      // variant is covered by a neutral constructed fixture, not a live capture.
      // Read explicit complete fields only; Material never supplies a title.
      for(const column of card.columns){
        let start=column.frame.start;
        for(const end of [...column.breaks,column.frame.end]){
          const field=/^(Creator|Source|Material|Member|Author|Pixiv ID|Twitter ID|Danbooru ID|Gelbooru ID|Characters):\s*(.*)$/.exec(tidy(plain.slice(start,end),MAX_TEXT));
          if(field){
            const value=field[2],anchors=column.links.filter(anchor=>anchor.start>=start&&anchor.end<=end);
            if(value&&value.length<=1024&&!/(?:^|\s)(?:Creator|Source|Material|Member|Author):/.test(value)&&metadata.length<10)metadata.push({label:field[1],value});
            const linked=anchors.length===1?sourceLinks.get(anchors[0]):null;
            if(field[1]==='Creator'){
              const creator=value&&value.length<=256&&!/(?:^|\s)(?:Creator|Source|Material|Member|Author):/.test(value)
                &&(!anchors.length||linked?.label===value)?value:null;
              explicitCreators.push(creator);creators.push(creator);
              creatorLinks.push(creator?linked:null);
            }else if(field[1]==='Source')sources.push(linked&&linked.label===value?linked:null);
          }
          start=end;
        }
      }
      const creator=creators.length===1?creators[0]:null;
      // Actual NASA-query response: titleless cards can carry database-post
      // links as visible source icons beside the score. Keep those observed
      // hrefs, not a guessed artist/title or a link derived from icon names.
      const iconLinks=[];
      for(const anchor of card.misc[0]?.links??[]){
        const url=anchor.anchor?.url;if(!url||!anchor.sourceIcon)continue;
        const parsed=new URL(url);if(/(^|\.)saucenao\.com$/.test(parsed.hostname))continue;
        if(!iconLinks.some(item=>item.url===url)&&iconLinks.length<3)iconLinks.push({label:textOf(anchor,256)||parsed.hostname,url});
      }
      if(!text){
        // A missing title is not a license to promote arbitrary links. Require
        // one unambiguous explicit creator and source in this same valid card.
        if(iconLinks.length)links=iconLinks;
        else {
          if(explicitCreators.length!==1||!creator||sources.length!==1||!sources[0])continue;
          const source=sources[0],profile=creatorLinks[0];
          links=[source];
          if(profile&&profile.url!==source.url)links.push(profile);
        }
      }else{
        for(const source of iconLinks)if(links.length<3&&!links.some(item=>item.url===source.url))links.push(source);
      }
      if(!links.length)continue;
      resultRanges.push([card.frame.start,card.frame.end]);
      if(result.candidates.length>=10||seen.has(links[0].url))continue;
      const candidate={title:text,creator,similarity,links};
      if(card.images.length===1)candidate.thumbnailSource=card.images[0];
      if(metadata.length)candidate.sourceMetadata=metadata;
      seen.add(links[0].url);result.candidates.push(candidate);
    }
    if(result.candidates.length)result.state='complete';
  }
  // Source titles and metadata can themselves discuss CAPTCHA or quotas. Only
  // remove text belonging to already-validated result links/cards, never the
  // whole main column or an unknown layout. Notices outside those ranges still
  // override results. Reuse bounded offsets from the single DOM projection.
  let noticeText='',offset=0;
  for(const [start,end] of resultRanges.sort((a,b)=>a[0]-b[0])){
    if(!Number.isInteger(start)||!Number.isInteger(end)||start<0||end<start||end>plain.length)continue;
    if(start>offset)noticeText+=plain.slice(offset,start)+' ';
    offset=Math.max(offset,end);
  }
  noticeText+=plain.slice(offset);
  const state=noticeState(tidy(noticeText,MAX_TEXT));
  if(state){result.state=state;result.candidates=[];}
  else if(sauceEnabled&&!result.candidates.length&&!sauceCards.length&&sauceLowNotices.length===1
    &&/^Low similarity results have been hidden\. Click here to display them(?:\.{3}|…)?$/i.test(textOf(sauceLowNotices[0],256))){
    // This is the original site's visible explanation, not an empty index or
    // a broken layout. Leave its hidden low-confidence cards untouched.
    result.state='lowSimilarityHidden';
  }
  else if(result.lens&&result.lens.mode!=='unknown'&&!result.candidates.length){
    const scope=mainFrames.length===1?mainFrames[0]:mainFrames.length===0&&emptyColumns.length===1?emptyColumns[0]:null;
    // A retained wrapper can represent an unparsed source even when it is not
    // a validated card. Keep zero conservative without admitting a new layout
    // or guessing its destination from query parameters.
    const unparsed=scope&&anchors.some(anchor=>anchor.visible&&scope.node.contains(anchor.node)
      &&(lensSource(anchor.url)||retainedLensRedirect(anchor.url)));
    if(scope&&!unparsed&&/(?:No exact matches(?: found)?|No matching results|No results found|정확히 일치하는 (?:검색)?결과가 없|일치하는 (?:검색)?결과가 없|(?:검색 결과|검색결과)가 없|일치검색 결과를 찾을 수 없음)/i.test(textOf(scope,MAX_TEXT)))result.state='empty';
  }
  return result;
})()
