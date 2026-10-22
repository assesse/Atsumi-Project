import {describe,expect,it} from 'vitest';
import {auditLayouts,layoutBrowser,type LayoutScene} from '../../test/offlineLayoutBrowser';
import css from './XWorkspace.css?raw';
const icon='<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 4h16v16H4z"/></svg>';
const header='<header class="x-toolbar x-clean-toolbar"><h1>X</h1><div class="x-feeds"><button aria-pressed="true">추천</button><button>팔로잉</button>'+Array.from({length:8},(_,i)=>'<button>계정별 커뮤니티 '+i+' 길이가 긴 제목</button>').join('')+'</div><div class="x-collections"><button>'+icon+'마음함</button><button>'+icon+'북마크</button></div><button class="x-refresh">'+icon+'</button><div class="x-account"><button>@sample_author ▾</button></div></header>';
const card='<article class="x-post"><button class="x-author-select" aria-label="작가 프로필 보기"><header><span class="x-avatar">작</span><div><strong>길이가 긴 작가 이름</strong><span>@sample_author</span></div><time>10월 18일</time></header></button><button class="x-post-select"><span class="x-text">앱에서 직접 그린 타임라인의 일반 게시물입니다.</span></button><div class="x-images count-1"><span class="x-photo-frame"><button class="x-photo">이미지</button></span></div><div class="x-actions">'+Array.from({length:5},()=>'<button>'+icon+'1.5만</button>').join('')+'</div></article>';
const screen=(detail=false)=>'<section class="x-workspace">'+header+'<div class="x-body"><div class="x-feed-column"><div class="x-filterbar"><input placeholder="불러온 게시물 검색"><button>미디어</button></div><div class="x-scroll"><div class="x-timeline">'+card.repeat(4)+'</div><div class="x-more"><button>더 보기</button></div></div></div><aside class="x-preview'+(detail?' has-selection':'')+'">'+(detail?'<header class="x-preview-heading"><span>게시물</span><button>×</button></header><div class="x-preview-scroll x-thread">'+card.repeat(3)+'</div>':'<p class="x-preview-empty">게시물을 선택하면 여기에 표시됩니다.</p>')+'</aside></div></section>';
const player='<div class="x-video-slot"><div class="x-inline-video"><video></video><div class="x-video-controls" role="group" aria-label="동영상 조작"><button aria-label="재생">'+icon+'</button><input type="range" aria-label="재생 위치"><span class="x-video-time">120:40 / 200:30</span><button aria-label="소리 켜기">'+icon+'</button><input class="x-volume" type="range" aria-label="음량"><button aria-label="동영상 전체 화면">'+icon+'</button></div></div></div>';
const imageViewer='<div class="x-image-backdrop"><div class="x-image-viewer" role="dialog" aria-modal="true" aria-label="이미지 크게 보기"><header><span>2 / 4</span><button aria-label="이미지 닫기">×</button></header><div class="x-full-image"><p>이미지 불러오는 중</p></div><footer><button aria-label="이전 이미지">‹</button><button aria-label="다음 이미지">›</button></footer></div></div>';
it('uses cover only in the preview and contain in the enlarged viewer',()=>{
  expect(css).toMatch(/\.x-detail-post \.x-photo img\{object-fit:cover\}/);
  expect(css).toMatch(/\.x-full-image img\{[^}]*object-fit:contain/);
});
describe.skipIf(!layoutBrowser)('clean-room X layout (separate from live acceptance)',()=>{
  it.each([[320,640],[720,480],[960,640],[1920,1080]])('keeps the native-free timeline reachable at %ix%i',async(width,height)=>{
    const scenes:LayoutScene[]=[
      {name:'timeline',html:screen()},
      {name:'detail',html:screen(true)},
      {name:'owned-video-controls',html:screen(true).replaceAll(card,'<article class="x-post">'+player+'</article>')},
      {name:'author-profile',html:screen(true).replace('<div class="x-filterbar">','<div class="x-profile-heading"><button aria-label="이전 피드로">←</button><strong>@sample_author</strong></div><div class="x-filterbar">')},
      {name:'thread-sections',html:screen(true).replace('<div class="x-preview-scroll x-thread">','<div class="x-preview-scroll x-thread"><section class="x-thread-section" aria-label="분류 미확인 게시물"><h2>분류 미확인 게시물</h2>'+card+'</section>')},
      {name:'image-viewer',html:screen(true).replace('</section>',imageViewer+'</section>')},
    ];
    const results=await auditLayouts(scenes,'html,body{height:100%;margin:0;font:14px sans-serif}*{box-sizing:border-box}'+css,width,height,'x-cleanroom-'+width);
    expect(results.flatMap(r=>r.issues.map(i=>r.name+': '+i))).toEqual([]);
  },60000);
});
