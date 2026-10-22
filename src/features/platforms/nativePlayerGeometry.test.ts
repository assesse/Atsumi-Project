import { afterEach, describe, expect, it } from 'vitest';
import { measureNativePlayerGeometry } from './nativePlayerGeometry';
function box(x: number, y: number, width: number, height: number, attributes = '') {
  const el = document.createElement('div'); el.innerHTML = attributes;
  el.getBoundingClientRect = () => ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON: () => ({}) });
  document.body.append(el); return el;
}
afterEach(() => document.body.replaceChildren());
const state = { sequence: 1, visible: true, muted: false };
describe('provider-neutral native geometry', () => {
  it('measures two separated surfaces without merging the gap or changing audio', () => {
    const a = box(0, 0, 300, 200), b = box(500, 0, 150, 200);
    expect(measureNativePlayerGeometry(a, b, state)).toEqual({ ...state, player: { x: 0, y: 0, width: 300, height: 200 }, chat: { x: 500, y: 0, width: 150, height: 200 }, occlusions: [] });
  });
  it('inactive pixels do not imply mute, while privacy mute is retained explicitly', () => {
    const a = box(0, 0, 300, 200);
    expect(measureNativePlayerGeometry(a, null, { ...state, visible: false })).toMatchObject({ visible: false, muted: false, player: null });
    expect(measureNativePlayerGeometry(a, null, { ...state, visible: false, muted: true })).toMatchObject({ visible: false, muted: true });
  });
  it('clips a partially scrolled player without resizing its original layout box', () => {
    const a = box(20, -50, 400, 300);
    const measured = measureNativePlayerGeometry(a, null, state);
    expect(measured.player).toEqual({ x:20, y:-50, width:400, height:300 });
    expect(measured.occlusions).toContainEqual({ x:20, y:-50, width:400, height:50 });
  });
  it('uses exact overlay geometry and four corner radii without shadow padding', () => {
    const a = box(0, 0, 600, 500), panel = box(100, 200, 300, 200);
    panel.dataset.platformNativeOcclusion = 'true';
    // jsdom does not expand border-radius shorthand into computed longhands.
    panel.style.borderTopLeftRadius = '18px'; panel.style.borderTopRightRadius = '18px';
    panel.style.borderBottomRightRadius = '0px'; panel.style.borderBottomLeftRadius = '0px';
    expect(measureNativePlayerGeometry(a, null, state).occlusions).toEqual([{ x: 100, y: 200, width: 300, height: 200, radii: [18, 18, 0, 0] }]);
  });
  it('hides native pixels/input behind modal dialogs but preserves audio', () => {
    const a = box(0, 0, 600, 500), dialog = box(100, 100, 300, 200);
    dialog.setAttribute('role', 'dialog'); dialog.setAttribute('aria-modal', 'true');
    expect(measureNativePlayerGeometry(a, null, state)).toMatchObject({ visible: false, muted: false });
  });
  it('ignores hidden foreign-platform overlays and hidden/inert surfaces', () => {
    const a = box(0, 0, 600, 500), dialog = box(100, 100, 300, 200);
    dialog.setAttribute('role', 'dialog'); dialog.setAttribute('aria-modal', 'true'); dialog.hidden = true;
    expect(measureNativePlayerGeometry(a, null, state).visible).toBe(true);
    a.setAttribute('inert', ''); expect(measureNativePlayerGeometry(a, null, state).visible).toBe(false);
  });
  it('lets a floating detail own its native slots but not render through another modal', () => {
    const detail = box(0, 0, 800, 600);
    detail.setAttribute('role', 'dialog'); detail.setAttribute('aria-modal', 'true');
    const player = box(0, 0, 500, 500), chat = box(510, 0, 200, 500);
    detail.append(player, chat);
    expect(measureNativePlayerGeometry(player, chat, state)).toMatchObject({ visible:true, muted:false, occlusions:[] });
    const foreground = box(100, 100, 300, 200);
    foreground.setAttribute('role', 'dialog'); foreground.setAttribute('aria-modal', 'true');
    expect(measureNativePlayerGeometry(player, chat, state)).toMatchObject({ visible:false, muted:false });
    foreground.remove(); document.body.append(chat);
    expect(measureNativePlayerGeometry(player, chat, state).visible).toBe(false);
  });
  it('fails closed on excessive overlay holes', () => {
    const a = box(0, 0, 600, 500);
    for (let i = 0; i < 17; i++) box(i, i, 20, 20).dataset.platformNativeOcclusion = 'true';
    expect(measureNativePlayerGeometry(a, null, state)).toMatchObject({ visible: false, occlusions: [] });
  });
  it('does not leave black native holes where added controls have faded out', () => {
    const a = box(0, 0, 600, 500), controls = box(100, 100, 120, 32);
    controls.dataset.platformNativeOcclusion = 'true'; controls.style.opacity = '0';
    expect(measureNativePlayerGeometry(a, null, state).occlusions).toEqual([]);
    controls.style.opacity = '1';
    expect(measureNativePlayerGeometry(a, null, state).occlusions).toHaveLength(1);
  });
  it('full-window ownership ignores outer grid clipping and hides other native panes', () => {
    const cell = box(0, 0, 200, 100); cell.style.overflow = 'hidden';
    Object.defineProperties(cell, { clientWidth:{value:200}, clientHeight:{value:100} });
    const owner = box(0, 0, 900, 600); owner.dataset.platformNativeFullscreen = 'true'; cell.append(owner);
    const a = box(0, 0, 700, 600); owner.append(a);
    expect(measureNativePlayerGeometry(a, null, state).player).toEqual({ x:0, y:0, width:700, height:600 });
    expect(measureNativePlayerGeometry(a, null, state).occlusions).toEqual([]);
    const foreign = box(0, 0, 300, 200);
    expect(measureNativePlayerGeometry(foreign, null, state)).toMatchObject({ visible:false, muted:false, player:null });
    const second = box(0, 0, 500, 400); second.dataset.platformNativeFullscreen = 'true';
    expect(measureNativePlayerGeometry(a, null, state).visible).toBe(false);
  });
  it('a fixed floating detail escapes the outer workspace but preserves its own scroll clip', () => {
    const workspace = box(214, 0, 800, 600); workspace.style.overflowX = workspace.style.overflowY = 'hidden';
    Object.defineProperties(workspace, {clientWidth:{value:800},clientHeight:{value:600}});
    const backdrop = box(0, 0, 1000, 600); backdrop.style.position = 'fixed'; workspace.append(backdrop);
    const detail = box(34, 100, 500, 400); detail.style.overflowX = detail.style.overflowY = 'auto'; backdrop.append(detail);
    Object.defineProperties(detail, {clientWidth:{value:500},clientHeight:{value:400}});
    const player = box(34, 100, 700, 400); detail.append(player);
    expect(measureNativePlayerGeometry(player, null, state)).toMatchObject({player:{x:34,y:100,width:700,height:400},occlusions:[{x:534,y:100,width:200,height:400}]});
  });
  it('fixed content still obeys its transformed containing block, skipping intermediate overflow', () => {
    const containing = box(214, 100, 400, 400); containing.style.transform = 'translateZ(0)'; containing.style.overflowX = containing.style.overflowY = 'hidden';
    Object.defineProperties(containing, {clientWidth:{value:400},clientHeight:{value:400}});
    const intermediate = box(314, 100, 100, 100); intermediate.style.overflowX = intermediate.style.overflowY = 'hidden'; containing.append(intermediate);
    Object.defineProperties(intermediate, {clientWidth:{value:100},clientHeight:{value:100}});
    const player = box(34, 100, 600, 300); player.style.position = 'fixed'; intermediate.append(player);
    expect(measureNativePlayerGeometry(player, null, state)).toMatchObject({player:{x:34,y:100,width:600,height:300},occlusions:[{x:34,y:100,width:180,height:300},{x:614,y:100,width:20,height:300}]});
  });
});
