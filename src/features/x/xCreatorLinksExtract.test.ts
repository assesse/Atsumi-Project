import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_creator_links_extract.js?raw';

const name = '<div data-testid="UserName"><span>Display name</span><span>@creator</span></div>';
const bio = (links: string) => `<div data-testid="UserDescription">Profile bio ${links}</div>`;
const anchor = (href: string, text = 'pixiv', extra = '') => `<a href="${href}" ${extra}>${text}</a>`;
function run(header: string, request: Record<string, unknown> = {}, path = '/creator', extra = '') {
  const doc = new DOMParser().parseFromString(`<div id="root" inert aria-hidden="true"><div data-testid="primaryColumn">${header}</div>${extra}</div>`, 'text/html');
  return evaluate(doc, request, path);
}
function evaluate(doc: Document, request: Record<string, unknown> = {}, path = '/creator') {
  const win = { top: null as unknown, getComputedStyle: (node: Element) => window.getComputedStyle(node) };
  win.top = win;
  const result = new Function('window', 'document', 'location', 'URL', `return (${script.replace('__ATSUMI_X_CREATOR_REQUEST__', JSON.stringify({ nonce: 'n', handle: 'creator', ...request }))});`)(win, doc, { protocol: 'https:', hostname: 'x.com', pathname: path, port: '' }, URL);
  return { result, doc };
}
afterEach(() => vi.restoreAllMocks());

it('reads only exact profile header bio and website links in their original order', () => {
  const { result } = run(name + bio(anchor('https://www.pixiv.net/users/123')) + `<div data-testid="UserUrl">${anchor('https://example.com/portfolio', 'website')}</div>`);
  expect(result).toEqual({ nonce: 'n', state: 'ready', profileUrl: 'https://x.com/creator', links: [
    { label: 'pixiv', url: 'https://www.pixiv.net/users/123' }, { label: 'website', url: 'https://example.com/portfolio' },
  ], unresolvedShortLinks: false });
});
const staticHeader = (links = anchor('http://pixiv.net/users/123', 'pixiv.net/users/123')) => `<div class="@container mb-4 flex flex-col gap-3 px-4 [--profile-avatar-size:clamp(48px,25cqw,136px)]">
  <div class="flex flex-col gap-1"><div class="min-w-0 flex flex-col items-start"><span class="flex max-w-full min-w-0 shrink-0 items-center gap-1"><h1 class="text-headline1">Fixture artist</h1><div>badge</div></span><span>@creator</span></div><div>profile controls</div></div>
  <div class="max-w-full text-body font-normal text-primary whitespace-pre-wrap wrap-break-word"><span>Profile bio</span><span>${links}</span></div>
  <div class="flex flex-wrap items-center gap-x-3 gap-y-1"><div>${anchor('https://t.co/website', 'example.com')}</div></div>
  <div class="flex flex-wrap items-center gap-x-5">${anchor('https://example.com/not-profile-evidence', 'following')}</div>
</div>`;
function staticDocument(header = staticHeader(), extra = '', before = '') {
  return `<html><head><link rel="canonical" href="https://x.com/creator"><meta property="og:url" content="https://x.com/creator"></head><body><div id="root" inert aria-hidden="true"><main>${before}<div class="flex flex-col">${header}</div><article><h2>Fixture post, not read</h2>${anchor('https://pixiv.net/users/999')}</article></main>${extra}</div></body></html>`;
}
const runStatic = (html = staticDocument(), request: Record<string, unknown> = {}) => evaluate(new DOMParser().parseFromString(html, 'text/html'), request).result;

it('reads the observed public static profile header without app test IDs', () => {
  expect(runStatic()).toMatchObject({ state: 'ready', profileUrl: 'https://x.com/creator', links: [
    { label: 'pixiv.net/users/123', url: 'http://pixiv.net/users/123' }, { label: 'example.com', url: 'https://t.co/website' },
  ], unresolvedShortLinks: true });
});
it('requires exact canonical URL and open-graph binding for the static variant', () => {
  for (const html of [
    staticDocument().replace('href="https://x.com/creator"', 'href="https://x.com/other"'),
    staticDocument().replace('content="https://x.com/creator"', 'content="https://x.com/other"'),
    staticDocument().replace('<meta property="og:url"', '<meta property="other"'),
  ]) expect(runStatic(html).state).not.toBe('ready');
});
it('requires the visible expected handle in the observed sibling identity group', () => {
  expect(runStatic(staticDocument().replace('<span>@creator</span>', '<span>@other</span>')).state).toBe('unavailable');
  expect(runStatic(staticDocument().replace('<span>@creator</span>', '<span hidden>@creator</span>')).state).toBe('unavailable');
  expect(runStatic(staticDocument().replace('<h1 class=', '<h1 hidden class=')).state).not.toBe('ready');
  expect(runStatic(staticDocument(staticHeader() + '<h1>Another profile</h1>')).state).not.toBe('ready');
});
it('rejects missing profile-container evidence and profile-shaped content in a post or sidebar', () => {
  expect(runStatic(staticDocument().replace('@container', 'container')).state).toBe('unavailable');
  expect(runStatic(staticDocument(`<article>${staticHeader()}</article>`)).state).not.toBe('ready');
  expect(runStatic(staticDocument('', `<aside>${staticHeader()}</aside>`)).state).not.toBe('ready');
  expect(runStatic(staticDocument(staticHeader(), '', '<article>Earlier post</article>')).state).toBe('unavailable');
});
it('excludes unrelated links before and after the header and the follow-count row', () => {
  const unknown = anchor('https://example.com/unrelated');
  const result = runStatic(staticDocument(staticHeader(), `<aside>${unknown}</aside>${unknown}`, unknown));
  expect(result.links.map((link: { url: string }) => link.url)).toEqual(['http://pixiv.net/users/123', 'https://t.co/website']);
});
it('never accepts embedded tweet/recommendation containers inside static profile evidence', () => {
  expect(runStatic(staticDocument(staticHeader(`<div data-testid="UserCell">${anchor('https://pixiv.net/users/999')}</div>`))).state).toBe('unavailable');
});
it('allows explicitly public static metadata without claiming authentication but rejects a visible owner mismatch', () => {
  expect(runStatic(staticDocument(), { owner: 'my_account' }).state).toBe('ready');
  const foreign = '<a data-testid="AppTabBar_Profile_Link" href="/other"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
  expect(runStatic(staticDocument(staticHeader(), foreign), { owner: 'my_account' }).state).toBe('ownerChanged');
});
it('does not use a whole-document anchor scan for either supported variant', () => {
  expect(script).not.toMatch(/document\.querySelectorAll\(['"]a(?:\[|['"])/);
});
it('keeps post, reply, recommendation and sidebar links out of the evidence', () => {
  const bad = `<div data-testid="UserDescription">${anchor('https://www.pixiv.net/users/999')}</div>`;
  const { result } = run(name + bio(anchor('https://www.pixiv.net/users/123')) + `<aside>${bad}</aside><div data-testid="UserCell">${bad}</div><article>${bad}</article>${bad}`);
  expect(result.links).toEqual([{ label: 'pixiv', url: 'https://www.pixiv.net/users/123' }]);
});
it('does not infer a profile identity from a source path alone or hidden/conflicting names', () => {
  expect(run(bio(anchor('https://www.pixiv.net/users/123'))).result.state).toBe('pending');
  expect(run(name.replace('@creator', '@other')).result.state).toBe('unavailable');
  expect(run(name, {}, '/other').result.state).toBe('unavailable');
  expect(run(name + name).result.state).toBe('pending');
  expect(run(name.replace('@creator', '<span hidden>@creator</span>')).result.state).toBe('unavailable');
});
it('distinguishes explicit no-links from a profile that has not loaded', () => {
  expect(run(name + '<div data-testid="UserDescription">No website</div>').result).toMatchObject({ state: 'ready', links: [] });
  expect(run('').result.state).toBe('pending');
});
it('uses an observed full expansion but never guesses a shortened destination', () => {
  const { result } = run(name + bio(anchor('https://t.co/expanded', 'pixiv.net/users/1…', 'title="https://www.pixiv.net/users/123"') + anchor('https://t.co/unknown', 'pixiv.net/users/4…')));
  expect(result.links).toEqual([{ label: 'pixiv.net/users/1…', url: 'https://www.pixiv.net/users/123' }, { label: 'pixiv.net/users/4…', url: 'https://t.co/unknown' }]);
  expect(result.unresolvedShortLinks).toBe(true);
});
it('rejects non-web, local, credentialed, hidden and internal X links', () => {
  const values = ['javascript:alert(1)', 'file:///tmp/file', 'http://127.0.0.1/file', 'https://[::1]/', 'https://user:pass@pixiv.net/', 'https://site.local/', 'https://site.test/', 'https://x.com/other', 'https://example.com:444/'];
  const { result } = run(name + bio(values.map(value => anchor(value)).join('') + `<span hidden>${anchor('https://example.com/hidden')}</span><span aria-hidden="true">${anchor('https://example.com/aria')}</span><span style="opacity:0">${anchor('https://example.com/transparent')}</span>`));
  expect(result.links).toEqual([]);
});
it('checks active account identity when a known profile owner was supplied', () => {
  const owner = '<a data-testid="AppTabBar_Profile_Link" href="/my_account"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
  expect(run(name, { owner: 'my_account' }, '/creator', owner).result.state).toBe('ready');
  expect(run(name, { owner: 'other' }, '/creator', owner).result.state).toBe('ownerChanged');
  expect(run(name, { owner: 'my_account' }).result.state).toBe('pending');
});
it('deduplicates and bounds declared links without replacing provider candidates', () => {
  const entries = Array.from({ length: 20 }, (_, i) => anchor(`https://example.com/${i}`, 'x'.repeat(200))).join('');
  const { result } = run(name + bio(anchor('https://example.com/0', 'first') + entries));
  expect(result.links).toHaveLength(8);
  expect(result.links[0].label).toBe('first');
  expect(result.links[1].label).toHaveLength(128);
  expect(result).not.toHaveProperty('creator');
  expect(result).not.toHaveProperty('similarity');
});
it('never reads credentials, tweet images, private application state or performs network requests', () => {
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql|querySelector(All)?\(['"](?:img|video)/);
});
it('bounds emoji labels without splitting UTF-16 surrogate pairs', () => {
  const result = run(name + bio(anchor('https://example.com/', 'a'.repeat(127) + '😀'))).result;
  expect(result.links[0].label).toBe('a'.repeat(127));
});
