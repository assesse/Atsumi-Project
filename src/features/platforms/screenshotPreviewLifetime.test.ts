import { it as test, expect } from 'vitest';
const assert = { equal: (actual: unknown, expected: unknown) => expect(actual).toBe(expected) };
import ts from './screenshotPreviewLifetime.ts?raw';
import native from '../../../src-tauri/src/streaming/browser_player_ui.js?raw';
import { createPreviewLifetime } from './screenshotPreviewLifetime';
function fixture(deadline = 45000) {
  let now = 0, next = 0, dismissed = 0, expired = 0;
  const tasks = new Map<number,{at:number;fn:()=>void}>(), issued: Array<()=>void> = [];
  const clock = { now: () => now, set(fn:()=>void, ms:number) { const id = ++next; tasks.set(id,{at:now+ms,fn}); issued.push(fn); return id as unknown as ReturnType<typeof setTimeout>; }, clear(id:ReturnType<typeof setTimeout>) { tasks.delete(id as unknown as number); } };
  const life = createPreviewLifetime(clock,deadline,{dismiss(){dismissed++;},expire(){expired++;}});
  const advance = (ms:number) => { const until=now+ms; for (;;) { const row=[...tasks].filter(([,v])=>v.at<=until).sort((a,b)=>a[1].at-b[1].at)[0]; if(!row)break; now=row[1].at;tasks.delete(row[0]);row[1].fn(); } now=until; };
  return {life,advance,issued,clock,get dismissed(){return dismissed;},get expired(){return expired;},get pending(){return tasks.size;}};
}
test('normal preview dismisses at3sec, no storage action exists',()=>{const f=fixture();f.advance(2999);assert.equal(f.dismissed,0);f.advance(1);assert.equal(f.dismissed,1);assert.equal(f.pending,0);});
test('hover holds and leave gives1.5sec',()=>{const f=fixture();f.advance(2000);f.life.hover(true);f.advance(10000);assert.equal(f.dismissed,0);f.life.hover(false);f.advance(1499);assert.equal(f.dismissed,0);f.advance(1);assert.equal(f.dismissed,1);});
test('focus and pointer hold independently; internal focus remains held',()=>{const f=fixture();f.life.hover(true);f.life.focus(true);f.life.focus(true);f.life.hover(false);f.advance(5000);assert.equal(f.dismissed,0);f.life.focus(false);f.advance(1500);assert.equal(f.dismissed,1);});
test('reenter cancels leave callback; stale timer cannot dismiss',()=>{const f=fixture();f.life.hover(true);f.life.hover(false);const stale=f.issued.at(-1);f.advance(1000);f.life.hover(true);stale!();f.advance(6000);assert.equal(f.dismissed,0);});
test('hover is capped at conservative deadline',()=>{const f=fixture();f.life.hover(true);f.advance(45000);assert.equal(f.expired,1);assert.equal(f.dismissed,1);});
test('deadline accounts for previous capture time, not preview arrival',()=>{const f=fixture(500);f.life.hover(true);f.advance(500);assert.equal(f.expired,1);assert.equal(f.dismissed,1);});
test('focus expiry preserves focus target, then1.5sec grace',()=>{const f=fixture();f.life.focus(true);f.advance(45000);assert.equal(f.expired,1);assert.equal(f.dismissed,0);f.life.focus(false);f.advance(1499);assert.equal(f.dismissed,0);f.advance(1);assert.equal(f.dismissed,1);});
test('busy or error blocks hide even across deadline',()=>{const f=fixture();f.life.block(true);f.advance(60000);assert.equal(f.dismissed,0);assert.equal(f.expired,1);f.life.block(false);f.advance(0);assert.equal(f.dismissed,1);});
test('dispose neutralizes every old callback without dismissal',()=>{const f=fixture();f.life.dispose();for(const fn of f.issued)fn();f.advance(100000);assert.equal(f.dismissed,0);assert.equal(f.expired,0);});
test('capture instances have no shared state',()=>{const a=fixture(),b=fixture();a.life.hover(true);a.advance(4000);b.advance(4000);assert.equal(a.dismissed,0);assert.equal(b.dismissed,1);});
test('focusout inside hover does not bypass hold',()=>{const f=fixture();f.life.hover(true);f.life.focus(true);f.life.focus(false);f.advance(10000);assert.equal(f.dismissed,0);f.life.hover(false);f.advance(1500);assert.equal(f.dismissed,1);});
test('native and typed presentation clocks have identical behavior source',()=>{
  const typed=ts.split('\n').slice(2).join('\n')
    .replace('export function','function')
    .replace('clock: PreviewClock, deadline: number, handlers: PreviewHandlers','clock, deadline, handlers')
    .replace('let hideTimer: ReturnType<typeof setTimeout> | null = null, expiryTimer: ReturnType<typeof setTimeout> | null = null, revision = 0;','let hideTimer = null, expiryTimer = null, revision = 0;')
    .replaceAll('(value: boolean) {','(value) {').trim();
  const start=native.indexOf('  function createPreviewLifetime('),end=native.indexOf('  const channel =',start);
  assert.equal(native.slice(start,end).split('\n').map(line=>line.startsWith('  ')?line.slice(2):line).join('\n').trim(),typed);
});
