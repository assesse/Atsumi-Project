import {expect, it, vi} from 'vitest';
import script from '../../../src-tauri/src/x_reader_accessibility.js?raw';
import projection from '../../../src-tauri/src/x_projection.js?raw';
import renderedMedia from '../../../src-tauri/src/x_rendered_media.js?raw';
import action from '../../../src-tauri/src/x_action.js?raw';
import intent from '../../../src-tauri/src/x_video_intent.js?raw';

const markup = '<div id="react-root"><a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn"><div role="tablist"><a role="tab" aria-selected="true">For you</a><a role="tab">Following</a></div><article data-testid="tweet"><div data-testid="User-Name"><a href="/artist">Artist</a><a href="/artist/status/123456789"><time datetime="2026-10-18T00:00:00Z"></time></a></div><div data-testid="tweetText">Fixture</div><button data-testid="like">1</button><div data-testid="videoPlayer"><video poster="https://pbs.twimg.com/amplify_video_thumb/987654321/img/Fixture.jpg"></video></div></article></main></div><div id="portal"></div>';
function environment(html = markup, options: {hostname?: string; protocol?: string; port?: string; child?: boolean; noBody?: boolean; diagnostics?: boolean; beforeInit?: (doc:Document)=>void} = {}) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  if (options.noBody) doc.body.remove();
  const win = Object.assign(new EventTarget(), {top: null as unknown, innerHeight: 900, scrollBy: vi.fn()});
  win.top = options.child ? {} : win;
  const location = {protocol: options.protocol ?? 'https:', hostname: options.hostname ?? 'x.com', port: options.port ?? '', origin: 'https://x.com', pathname: '/home'};
  const evaluate = (source: string) => new Function('window', 'document', 'location', 'return (' + source + ');')(win, doc, location);
  options.beforeInit?.(doc);
  evaluate(script.replace('__ATSUMI_X_READER_DIAGNOSTICS__', options.diagnostics === false ? 'false' : 'true'));
  return {doc, win, evaluate};
}
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };

it('covers actual content roots rather than invalid html/body aria-hidden', () => {
  const {doc} = environment();
  expect(doc.documentElement.hasAttribute('aria-hidden')).toBe(false);
  expect(doc.body.hasAttribute('aria-hidden')).toBe(false);
  expect([...doc.body.children].map(root => root.getAttribute('aria-hidden'))).toEqual(['true', 'true']);
  expect(doc.querySelector('article')).not.toBeNull();
  expect([...doc.body.children].every(root => root.hasAttribute('inert'))).toBe(true);
  expect(doc.querySelectorAll('[hidden],[style],[disabled]')).toHaveLength(0);
  expect(doc.documentElement.hasAttribute('inert')).toBe(false);
  expect(doc.body.hasAttribute('inert')).toBe(false);
});

it('covers a body and root created after early document initialization', async () => {
  const {doc} = environment('', {noBody: true});
  const body = doc.createElement('body');
  body.innerHTML = '<div id="late-root"><button>Fixture</button></div>';
  doc.documentElement.append(body);
  await settle();
  expect(doc.getElementById('late-root')?.getAttribute('aria-hidden')).toBe('true');
  expect(doc.getElementById('late-root')?.hasAttribute('inert')).toBe(true);
  expect(body.hasAttribute('aria-hidden')).toBe(false);
});

it('covers newly inserted portals and replaced roots without moving descendants', async () => {
  const {doc} = environment();
  const replacement = doc.createElement('section');
  replacement.innerHTML = '<button id="retained">Fixture</button>';
  const button = replacement.firstElementChild;
  doc.getElementById('react-root')!.replaceWith(replacement);
  const portal = doc.createElement('aside');
  doc.body.append(portal);
  await settle();
  expect(replacement.getAttribute('aria-hidden')).toBe('true');
  expect(portal.getAttribute('aria-hidden')).toBe('true');
  expect(replacement.hasAttribute('inert')).toBe(true);
  expect(portal.hasAttribute('inert')).toBe(true);
  expect(replacement.firstElementChild).toBe(button);
  const nextBody = doc.createElement('body');
  nextBody.innerHTML = '<main id="new-body-root"></main>';
  doc.body.replaceWith(nextBody);
  await settle();
  expect(doc.getElementById('new-body-root')?.getAttribute('aria-hidden')).toBe('true');
  expect(doc.getElementById('new-body-root')?.hasAttribute('inert')).toBe(true);
});

it('repairs root resets idempotently and does not rewrite nested site accessibility', async () => {
  const {doc} = environment();
  const root = doc.getElementById('react-root')!;
  const writes = vi.spyOn(root, 'setAttribute');
  root.removeAttribute('aria-hidden');
  await settle();
  expect(writes).toHaveBeenCalledExactlyOnceWith('aria-hidden', 'true');
  await settle();
  expect(writes).toHaveBeenCalledTimes(1);
  const article = doc.querySelector('article')!;
  article.setAttribute('aria-hidden', 'false');
  await settle();
  expect(article.getAttribute('aria-hidden')).toBe('false');
  expect(writes).toHaveBeenCalledTimes(1);
  root.setAttribute('aria-hidden', 'false');
  await settle();
  expect(root.getAttribute('aria-hidden')).toBe('true');
  expect(writes).toHaveBeenCalledTimes(3);
});

it.each([
  {hostname: 'example.test'}, {hostname: 'x.com.example.test'},
  {protocol: 'http:'}, {port: '444'}, {child: true},
])('does not run outside an exact top-level secure X reader context: %j', options => {
  const {doc} = environment(markup, options);
  expect(doc.querySelector('[aria-hidden]')).toBeNull();
});

it('disconnects when the reader document is discarded', async () => {
  const {doc, win} = environment();
  win.dispatchEvent(new Event('pagehide'));
  const root = doc.getElementById('react-root')!;
  root.removeAttribute('aria-hidden');
  root.removeAttribute('inert');
  await settle();
  expect(root.hasAttribute('aria-hidden')).toBe(false);
  expect(root.hasAttribute('inert')).toBe(false);
});

it('reconnects once after BFCache restoration across repeated pagehide/pageshow cycles', async () => {
  const {doc, win} = environment();
  const root = doc.getElementById('react-root')!;
  const writes = vi.spyOn(root, 'setAttribute');
  for (let cycle = 0; cycle < 2; cycle++) {
    win.dispatchEvent(new Event('pagehide'));
    root.removeAttribute('aria-hidden');
    root.removeAttribute('inert');
    await settle();
    expect(root.hasAttribute('aria-hidden')).toBe(false);
    win.dispatchEvent(new Event('pageshow'));
    win.dispatchEvent(new Event('pageshow'));
    await settle();
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(root.hasAttribute('inert')).toBe(true);
    expect(writes).toHaveBeenCalledTimes(cycle * 3 + 2);
    root.removeAttribute('aria-hidden');
    await settle();
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(writes).toHaveBeenCalledTimes(cycle * 3 + 3);
  }
});

it('preserves actual DOM projection, poster observation and programmatic controls', async () => {
  const {doc, win, evaluate} = environment();
  const project = () => evaluate(projection.replace('__ATSUMI_X_REQUEST__', JSON.stringify({nonce: 'test', feed: 'foryou', owner: 'alice'})));
  expect(project()).toMatchObject({state: 'ready', posts: [{id: '123456789', text: 'Fixture', video: true, stats: {liked: false, canLike: true}}]});
  evaluate(renderedMedia);
  const facts = (win as unknown as {__atsumiXRenderedMedia: {get: (owner: string, id: string) => string[]}}).__atsumiXRenderedMedia;
  expect(facts.get('alice', '123456789')).toEqual(['https://pbs.twimg.com/amplify_video_thumb/987654321/img/Fixture.jpg']);
  const button = doc.querySelector('[data-testid="like"]')!;
  const clicked = vi.fn(() => button.setAttribute('data-testid', 'unlike'));
  button.addEventListener('click', clicked);
  const request = {nonce: 'test', path: '/artist/status/123456789', documentPath: '/home', owner: 'alice', action: 'like', expected: false};
  expect(evaluate(action.replace('__ATSUMI_X_ACTION__', JSON.stringify({...request, phase: 'probe'})))).toMatchObject({state: 'ready'});
  expect(clicked).not.toHaveBeenCalled();
  expect(evaluate(action.replace('__ATSUMI_X_ACTION__', JSON.stringify(request)))).toMatchObject({state: 'sent'});
  expect(clicked).toHaveBeenCalledTimes(1);
  expect(project().posts[0].stats.liked).toBe(true);
  expect(evaluate(intent.replace('__ATSUMI_X_VIDEO__', JSON.stringify({...request, inspect: true})))).toMatchObject({state: 'observed', intentSent: false, video: true});
  const video = doc.querySelector('video')!;
  const play = vi.spyOn(video, 'play').mockResolvedValue();
  const article = doc.querySelector('article')!;
  article.scrollIntoView = vi.fn();
  expect(evaluate(intent.replace('__ATSUMI_X_VIDEO__', JSON.stringify({...request, inspect: false})))).toMatchObject({state: 'observed', intentSent: true, video: true});
  expect(play).toHaveBeenCalledTimes(1);
  await settle();
  expect(doc.getElementById('react-root')?.getAttribute('aria-hidden')).toBe('true');
});

it('does not change hydration, manually redirect focus, authenticate or use the network', () => {
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|\.focus\(|\.blur\(|\.click\(|\.play\(|display\s*[:=]|visibility\s*[:=]|innerHTML|outerHTML|replaceWith|appendChild/);
  expect(script).not.toMatch(/setAttribute\(['"](?:hidden|style|tabindex|disabled)['"]/);
});

it('makes a focus-retaining content root inert before setting aria-hidden', () => {
  let writes: ReturnType<typeof vi.spyOn>;
  const {doc, win} = environment(markup, {beforeInit: doc => {
    const focused = doc.querySelector('button')!;
    // A synthetic retained-focus condition, not a claim that JSDOM implements
    // Chromium's inert/focus or native accessibility behavior.
    Object.defineProperty(doc, 'activeElement', {get: () => focused});
    writes = vi.spyOn(doc.getElementById('react-root')!, 'setAttribute');
  }});
  expect(writes!.mock.calls.slice(0,2)).toEqual([['inert',''], ['aria-hidden','true']]);
  const diagnostics = (win as unknown as {__atsumiXReaderAccessibility:{snapshot:()=>Record<string,unknown>}}).__atsumiXReaderAccessibility.snapshot();
  expect(diagnostics).toMatchObject({installed:true,observing:true,roots:2,inertRoots:2,ariaHiddenRoots:2,focusInContent:true,focusedBeforeCover:true});
  expect(Object.values(diagnostics).every(value => typeof value === 'boolean' || typeof value === 'number')).toBe(true);
  expect(doc.querySelector('button')!.hasAttribute('disabled')).toBe(false);
});

it('repairs an inert reset without repeated writes or altering nested controls', async () => {
  const {doc} = environment();
  const root = doc.getElementById('react-root')!;
  const writes = vi.spyOn(root, 'setAttribute');
  root.removeAttribute('inert');
  await settle();
  expect(writes).toHaveBeenCalledExactlyOnceWith('inert','');
  await settle();
  expect(writes).toHaveBeenCalledTimes(1);
  expect(doc.querySelector('button')!.hasAttribute('inert')).toBe(false);
});

it('does not expose diagnostic snapshots in release initialization', () => {
  const {doc, win} = environment(markup, {diagnostics:false});
  expect('__atsumiXReaderAccessibility' in win).toBe(false);
  expect(doc.getElementById('react-root')?.hasAttribute('inert')).toBe(true);
});
