import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { prepareDemo, outputRoot, digest, toolsRoot } from './build.mjs';
import { byteRange, framePolicy, startServer } from './server.mjs';
import { pathToFileURL } from 'node:url';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { fixture } from './surface-data.js';
import { createChatSearchIndex, searchChatMessages, createReplaySearch } from './replay-search.js';

test('all original script and FULL stylesheet bytes match source hashes', async () => {
  const { proof, templates } = await prepareDemo();
  for (const original of proof.originalFiles) assert.equal(digest(await readFile(path.join(outputRoot, original.file))), original.sha256);
  assert.equal(proof.originalFiles.length, 6);
  for (const mode of ['live', 'vod']) {
    for (const key of ['custom__shop-button', 'custom__clip-button', 'setting-radio-mode', 'pzp-pc-pip-button', 'pzp-pc-viewmode-button', 'pzp-pc-fullscreen-button']) assert.ok(templates[mode].includes(key));
    assert.equal(digest(templates[mode]), proof.expressions[mode].htmlSha256);
  }
  assert.match(templates.liveBadge, /live_time_text/);
  assert.match(templates.liveBadge, /실시간/);
  assert.ok(!templates.liveBadge.includes('onClick'));
});
test('adapter does not delete original buttons or draw replacement controls', async () => {
  const source = await readFile(path.join(toolsRoot, 'frame.js'), 'utf8');
  assert.doesNotMatch(source, /\.remove\(|innerHTML\s*=|createElement\(['"](?:button|svg)/);
  const css = await readFile(path.join(toolsRoot, 'frame.css'), 'utf8');
  assert.doesNotMatch(css, /\.pzp-|font-family|color\s*:/);
});

test('live header uses original details in both widths and only the redundant mute notice is hidden', async () => {
  const source = await readFile(path.join(toolsRoot, 'frame.js'), 'utf8');
  assert.ok(source.includes('K.createElement(BroadcastInfo, { wide: true, chatHidden,'));
  assert.ok(source.includes('player.muted = true;'), 'hiding the notice must not enable audio');
  const css = (await readFile(path.join(toolsRoot, 'player-adaptations.css'), 'utf8')).replace(/\/\*[\s\S]*?\*\//g, '').trim();
  assert.equal(css, '.chzzk_player .pzp-pc__mute-indicator { display: none !important; }');
  const original = await readFile(path.join(outputRoot, 'local-index.css'), 'utf8');
  assert.ok(original.includes('.pzp-pc .player_header .header_info{opacity:0;'));
  assert.ok(original.includes('.pzp-pc.pzp-pc--controls .player_header .header_info{opacity:1}'));
});
test('opaque player document permits local resources but no remote connections', () => {
  const policy = framePolicy('http://127.0.0.1:18764');
  for (const directive of ["connect-src 'none'", "worker-src 'none'", "frame-src 'none'", 'sandbox allow-scripts']) assert.ok(policy.includes(directive));
  assert.ok(!policy.includes('allow-same-origin'));
  assert.ok(!policy.includes('https:'));
});

test('broadcast and LIVE chat expressions retain pinned source ranges and local assets', async () => {
  const { proof } = await prepareDemo();
  const index = await readFile(path.join(outputRoot, '../chzzk-original-player/index-C4sif-4p.js'), 'utf8');
  for (const record of proof.surfaces.records) assert.equal(digest(index.slice(...record.utf16Range)), record.sha256, record.name);
  for (const file of [...proof.resourceFiles, ...proof.derivedFiles]) assert.equal(digest(await readFile(path.join(outputRoot, file.file))), file.sha256);
  assert.equal(proof.resourceFiles.filter(file => file.file.endsWith('.woff2')).length, 121);
  const css = await readFile(path.join(outputRoot, 'local-index.css'), 'utf8');
  assert.match(css, /url\(\/assets\/SDNemony2dBasicBd115\.woff2\)/);
  assert.match(css, /url\(\/assets\/icon_official_mark\.png\)/);
  const module = await readFile(path.join(outputRoot, 'original-surfaces.js'), 'utf8');
  assert.doesNotMatch(module, /\bwg\s*\(/, 'share-plugin loading effect must not accompany the original menu return');
  assert.doesNotMatch(module, /\b(?:fetch|WebSocket|XMLHttpRequest|sendBeacon)\s*\(/);
});

test('original wide header and LIVE chat render with synthetic data, not app UI', async () => {
  await prepareDemo();
  const { createSurfaces } = await import(pathToFileURL(path.join(outputRoot, 'original-surfaces.js')));
  const previous = globalThis.window;
  globalThis.window = { self: {}, top: {} };
  try {
    // The real browser uses the original autosize-textarea export. SSR only
    // needs its intrinsic node; no mouse/layout/app hooks run in this test.
    const { BroadcastInfo, Chat } = createSurfaces(React, { createPortal: value => value }, props => React.createElement('textarea', props), fixture, () => {});
    const narrow = renderToStaticMarkup(React.createElement(BroadcastInfo, { wide: false }));
    const wide = renderToStaticMarkup(React.createElement(BroadcastInfo, { wide: true }));
    assert.ok(!narrow.includes(fixture.channelName));
    for (const text of [fixture.title, fixture.channelName, '현재 1,234', fixture.uptime, '/assets/default_profile_dark.png', '_detail_header_1m3jr_7']) assert.ok(wide.includes(text), text);
    const saved = renderToStaticMarkup(React.createElement(BroadcastInfo, { wide: true, replay: true, title: '저장본 <방송 이름>', channelName: '', recordedAt: 1789272000000 }));
    for (const text of ['저장본 &lt;방송 이름&gt;', '_detail_header_1m3jr_7', '2026.']) assert.ok(saved.includes(text), text);
    for (const text of ['현재 1,234', fixture.channelName, fixture.uptime, '>LIVE<', '스트리밍 중']) assert.ok(!saved.includes(text), text);
    const profileImage = 'data:image/png;base64,iVBORw0KGgo=';
    const savedProfile = renderToStaticMarkup(React.createElement(BroadcastInfo, { wide: true, replay: true, title: '저장 방송', channelName: '저장 채널', profileImage }));
    assert.ok(savedProfile.includes(profileImage)); assert.ok(savedProfile.includes('저장 채널'));
    const chat = renderToStaticMarkup(React.createElement(Chat));
    for (const text of ['aside-chatting', '_title_1e2su_12', 'role="log"', '_wrapper_8lqsk_25', '채팅에 참여하려면 로그인 해주세요', '후원하기', 'send_chat_or_donate', '초록빛']) assert.ok(chat.includes(text), text);
    assert.equal((chat.match(/class="_item_8lqsk_7"/g) || []).length, 60);
    assert.ok(!chat.includes('recording-replay-'));
    const replay = renderToStaticMarkup(React.createElement(Chat, { replay: true }));
    for (const text of ['aside-chatting', '_title_1e2su_12', 'role="log"', '_wrapper_8lqsk_25', 'type="search"', 'placeholder="검색"', 'aria-label="검색"', 'data-replay-search-area=""', '초록빛']) assert.ok(replay.includes(text), text);
    assert.doesNotMatch(replay, /검색 범위|60개 채팅|재생 위치/);
    for (const removed of ['후원하기', 'send_chat_or_donate', '_tools_19u4u_125', '채팅에 참여하려면 로그인 해주세요']) assert.ok(!replay.includes(removed), removed);
    assert.equal((replay.match(/class="_item_8lqsk_7"/g) || []).length, 60);
    assert.ok(!chat.includes('data-replay-search-area'), 'live footer must not receive replay styling');
    const { Chat: EmptyChat } = createSurfaces(React, { createPortal: value => value }, props => React.createElement('textarea', props), { ...fixture, messages: [] }, () => {});
    const emptyReplay = renderToStaticMarkup(React.createElement(EmptyChat, { replay: true }));
    assert.ok(emptyReplay.includes('저장된 채팅이 없습니다.'));
    assert.ok(!emptyReplay.includes('0개 채팅'));
    assert.ok(!renderToStaticMarkup(React.createElement(EmptyChat)).includes('replay-search'));
  } finally { if (previous === undefined) delete globalThis.window; else globalThis.window = previous; }
});

test('replay search filters the complete local data by literal body or nickname without modifying it', () => {
  const index = createChatSearchIndex(fixture.messages);
  assert.deepEqual(searchChatMessages(index, ''), fixture.messages);
  assert.deepEqual(searchChatMessages(index, '  \t '), fixture.messages);
  assert.equal(searchChatMessages(index, '달빛산책').length, 10);
  assert.equal(searchChatMessages(index, '달빛산책', 'nickname').length, 10);
  assert.equal(searchChatMessages(index, '달빛산책', 'body').length, 0);
  assert.equal(searchChatMessages(index, '줄 간격', 'body').length, 10);
  assert.equal(searchChatMessages(index, '줄 간격', 'nickname').length, 0);
  assert.equal(searchChatMessages(index, '검색되지 않는 문구').length, 0);
  assert.deepEqual(searchChatMessages(index, '달빛산책', 'invalid'), searchChatMessages(index, '달빛산책'));
  assert.equal(searchChatMessages(index, '달빛산책')[0], fixture.messages[1]);
  assert.equal(searchChatMessages(index, '달빛산책').at(-1), fixture.messages[55]);
  assert.equal(fixture.messages.length, 60);
});

test('search normalizes Korean/Latin text, treats metacharacters literally, and tolerates absent metadata', () => {
  const messages = [
    { content: ' Hello\nWorld [1].* ', profile: { nickname: 'Ｔｅｓｔ 한글' } },
    { content: null, profile: null }, { profile: {} },
  ];
  const index = createChatSearchIndex(messages);
  for (const query of ['hello world', ' HELLO   WORLD ', '[1].*']) assert.deepEqual(searchChatMessages(index, query, 'body'), [messages[0]]);
  for (const query of ['test 한글', '한글'.normalize('NFD')]) assert.deepEqual(searchChatMessages(index, query, 'nickname'), [messages[0]]);
  assert.deepEqual(searchChatMessages(index, '.*'), [messages[0]]);
  assert.deepEqual(searchChatMessages(index, '<img onerror=alert(1)>'), []);
  assert.deepEqual(searchChatMessages(index, ''), messages);
});

test('replay search footer is one combined search input without a filter/status row or posting form', () => {
  const Search = createReplaySearch(React, { container: 'original-container', input: 'original-input' });
  const html = renderToStaticMarkup(React.createElement(Search, {
    query: '<script>alert(1)</script>', count: 0, onQuery: () => {},
  }));
  for (const text of ['0개 결과', '검색 지우기', 'replay-search__announcement', 'original-input', 'type="search"', 'placeholder="검색"', 'aria-label="검색"']) assert.ok(html.includes(text), text);
  assert.doesNotMatch(html, /검색 범위|replay-search__meta|replay-search__fields|재생 위치/);
  const idle = renderToStaticMarkup(React.createElement(Search, { query: '', count: 60, onQuery: () => {} }));
  assert.doesNotMatch(idle, /role="status"|개 채팅|전체|본문|닉네임|재생 위치/);
  assert.ok(!html.includes('채팅 본문·닉네임 검색'));
  assert.doesNotMatch(html, /<script|<form|type="submit"/);
});

test('mode selection reaches both frames and search behavior never controls the player or network', async () => {
  const host = await readFile(path.join(toolsRoot, 'demo.js'), 'utf8');
  assert.ok(host.includes('frame.src = `/frame.html?mode=${mode}#${nonce}`'));
  assert.ok(host.includes('chat.src = `/chat.html?mode=${mode}#${chatNonce}`'));
  const chat = await readFile(path.join(toolsRoot, 'chat.js'), 'utf8');
  assert.ok(chat.includes("new URLSearchParams(location.search).get('mode') === 'vod'"));
  const search = await readFile(path.join(toolsRoot, 'replay-search.js'), 'utf8');
  assert.doesNotMatch(search, /\b(?:fetch|WebSocket|XMLHttpRequest|sendBeacon|postMessage)\s*\(|\.play\(|\.pause\(|\.currentTime\s*=/);
});
test('range validation supports seek and rejects malformed/unbounded ranges', () => {
  assert.deepEqual(byteRange('bytes=10-19', 100), { start: 10, end: 19, status: 206 });
  assert.deepEqual(byteRange('bytes=-10', 100), { start: 90, end: 99, status: 206 });
  for (const range of ['bytes=100-', 'bytes=20-10', 'bytes=-0', 'bytes=0-2,4-6', 'bytes=9007199254740992-']) assert.equal(byteRange(range, 100), null);
  assert.equal(byteRange('bytes=0-', 10000000).end, 1048575);
});
test('standalone bundle needs no build tool or source imports at runtime', async () => {
  await prepareDemo();
  const server = await startServer(0, outputRoot), origin = `http://127.0.0.1:${server.address().port}`;
  try {
    const page = await fetch(`${origin}/frame.html?mode=live`);
    assert.ok(page.headers.get('content-security-policy').includes("connect-src 'none'"));
    const playerHtml = await page.text();
    assert.ok(playerHtml.includes('live_player_layout'));
    assert.ok(playerHtml.includes('href="/player-adaptations.css"'));
    assert.equal((await fetch(`${origin}/player-adaptations.css`)).status, 200);
    const chat = await fetch(`${origin}/chat.html`);
    assert.ok(chat.headers.get('content-security-policy').includes("connect-src 'none'"));
    const chatHtml = await chat.text();
    assert.ok(chatHtml.includes('id="chat-root"'));
    assert.ok(!chatHtml.includes('replay-search.css'));
    assert.ok(!chatHtml.includes('player-adaptations.css'));
    const replayChat = await fetch(`${origin}/chat.html?mode=vod`);
    assert.ok(replayChat.headers.get('content-security-policy').includes("connect-src 'none'"));
    assert.ok((await replayChat.text()).includes('href="/replay-search.css"'));
    assert.equal((await fetch(`${origin}/replay-search.js`)).status, 200);
    assert.equal((await fetch(`${origin}/replay-search.css`)).status, 200);
    const font = await fetch(`${origin}/assets/SDNemony2dBasicBd115.woff2`);
    assert.equal(font.headers.get('content-type'), 'font/woff2');
    assert.equal(font.headers.get('access-control-allow-origin'), 'null');
    assert.equal(Buffer.from(await font.arrayBuffer()).subarray(0, 4).toString(), 'wOF2');
    for (const target of ['/package.json', '/src/App.tsx', '/%2e%2e/package.json', '/manifest.json/extra']) assert.equal((await fetch(origin + target)).status, 404);
    assert.equal((await fetch(origin, { method: 'POST' })).status, 403);
    const video = await fetch(`${origin}/sample.mp4`, { headers: { Range: 'bytes=0-15' } });
    assert.equal(video.status, 206);
    assert.equal((await video.arrayBuffer()).byteLength, 16);
    assert.equal((await fetch(`${origin}/sample.mp4`, { headers: { Range: 'bytes=-0' } })).status, 416);
  } finally { await new Promise(resolve => server.close(resolve)); }
});
