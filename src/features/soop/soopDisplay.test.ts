import { describe, expect, it } from 'vitest';
import type { SoopDisplayPalette, SoopDisplayState } from '../platforms/platformMedia';
import { soopDisplayLabel, soopPaletteStyle, validSoopPalette, validSoopSurface } from './soopDisplay';

const palette = (): SoopDisplayPalette => ({ version: 1, basis: 'computed-css-variable',
  nickname: Array.from({ length: 20 }, () => '#123456'),
  roles: { normal: '#222222', streamer: '#333333', manager: '#444444', vip: '#555555', subscribe: '#666666', fan: '#777777', support: '#888888', nightbot: '#999999' } });
const display = (colors?: SoopDisplayPalette): SoopDisplayState => ({ version: 1,
  values: { theme: 'dark', fontLevel: 2, randomNicknameColor: true, fanMessageColor: true, showEmoticons: true, animateEmoticons: true, smallOgq: false },
  basis: { theme: 'dom', fontLevel: 'dom', randomNicknameColor: 'dom', fanMessageColor: 'saved-option', showEmoticons: 'saved-option', animateEmoticons: 'saved-option', smallOgq: 'saved-option' },
  documentEpoch: 1, sequence: 1, observedAtMs: 1000, timeMs: 0, timingQuality: 'receive-time-approximate', palette: colors });

describe('saved SOOP palette boundary', () => {
  it('retains only the28 fixed variables and qualifies unknown legacy or partial colors', () => {
    const value = palette(), state = display(value);
    expect(validSoopPalette(value)).toBe(true);
    expect(Object.keys(soopPaletteStyle(state)!)).toHaveLength(28);
    expect(soopPaletteStyle(state)).toMatchObject({ '--nickname-ramdom-color1': '#123456', '--normal-color': '#222222' });
    expect(soopDisplayLabel(state)).toBe('저장 당시 표시 설정 · 색상 일부');
    value.nickname[0] = null;
    expect(validSoopPalette(value)).toBe(true);
    expect(soopPaletteStyle(state)).not.toHaveProperty('--nickname-ramdom-color1');
    expect(soopDisplayLabel(state)).toContain('색상 일부');
    expect(soopDisplayLabel(display())).toContain('색상 기록 없음');
    expect(soopDisplayLabel({ ...display(), paletteIncomplete: true })).toContain('색상 일부');
  });

  it.each(['#ABCDEF', '#abc', '#12345678', 'red', 'rgb(1,2,3)', 'var(--x)', 'url(https://example.test/a)', '#123456; color:red', '', undefined])('rejects an untrusted stored color %s', color => {
    const value = palette(); (value.nickname as unknown[])[2] = color;
    expect(validSoopPalette(value)).toBe(false);
    expect(soopPaletteStyle(display(value))).toBeUndefined();
  });

  it('rejects changed schema, incomplete role keys, sparse arrays and appended payload', () => {
    const variants: unknown[] = [null, { ...palette(), version: 2 }, { ...palette(), extra: '#000000' },
      { ...palette(), basis: 'dom' }, { ...palette(), nickname: Array(20) },
      { ...palette(), nickname: Array(21).fill(null) }, { ...palette(), roles: { normal: '#123456' } },
      { ...palette(), roles: { ...palette().roles, other: '#123456' } }];
    for (const value of variants) expect(validSoopPalette(value)).toBe(false);
  });

  it('keeps captured colors for font changes but never invents opposite-theme history', () => {
    const state = display(palette());
    expect(soopPaletteStyle(state, { fontLevel: 8 })).toEqual(soopPaletteStyle(state));
    expect(soopPaletteStyle(state, { theme: 'dark' })).toEqual(soopPaletteStyle(state));
    expect(soopPaletteStyle(state, { theme: 'light' })).toBeUndefined();
    expect(soopDisplayLabel(state, { theme: 'light' })).toBe('사용자 표시 설정');
    state.basis.theme = 'saved-option';
    expect(soopPaletteStyle(state)).toBeUndefined();
  });
});

describe('saved SOOP ordinary surface boundary', () => {
  const surface = () => ({ version: 1 as const, basis: 'computed-dom' as const, foreground: '#e2e4e9', background: '#0c0d0e' });
  it('preserves the pair independently of palette and scopes it to a matching observed theme', () => {
    const state = { ...display(), surface: surface() };
    expect(soopPaletteStyle(state)).toEqual({ '--soop-captured-foreground': '#e2e4e9', backgroundColor: '#0c0d0e' });
    expect(soopPaletteStyle(state, { theme: 'dark', fontLevel: 8 })).toEqual(soopPaletteStyle(state));
    expect(soopPaletteStyle(state, { theme: 'light' })).toBeUndefined();
    state.values.theme = null; expect(soopPaletteStyle(state)).toBeUndefined();
  });
  it('does not count the pair as author colors, discard palette, or carry former values into unknown history', () => {
    const state = { ...display(palette()), surface: surface() };
    expect(Object.keys(soopPaletteStyle(state)!)).toHaveLength(30);
    expect(soopDisplayLabel(state)).toBe('저장 당시 표시 설정');
    expect(soopDisplayLabel({ ...state, surfaceIncomplete: true })).toContain('색상 일부');
    expect(soopPaletteStyle({ ...state, surface: { ...surface(), foreground: null } })).toHaveProperty('backgroundColor', '#0c0d0e');
    expect(soopPaletteStyle({ ...state, surface: null })).not.toHaveProperty('--soop-captured-foreground');
    expect(soopPaletteStyle({ ...state, surface: null })).not.toHaveProperty('backgroundColor');
    expect(Object.keys(soopPaletteStyle({ ...state, surface: null })!)).toHaveLength(28);
  });
  it.each([undefined, null, {}, { ...surface(), extra: 'red' }, { ...surface(), basis: 'css' },
    { ...surface(), version: 2 }, { ...surface(), foreground: '#ABCDEF' }, { ...surface(), background: 'rgba(1,2,3,.5)' },
    { ...surface(), foreground: 'var(--x)' }, { ...surface(), background: 'url(https://example.test)' }])('isolates malformed optional surface while retaining valid author history (%j)', value => {
    expect(validSoopSurface(value)).toBe(false);
    const state = { ...display(palette()), surface: value } as SoopDisplayState;
    expect(Object.keys(soopPaletteStyle(state)!)).toHaveLength(28);
    expect(soopDisplayLabel(state)).toContain('색상 일부');
  });
});
