import type { CSSProperties } from 'react';
import type { SoopDisplayPalette, SoopDisplayState, SoopDisplaySurface, SoopDisplayValues, SoopRowDisplay } from '../platforms/platformMedia';

export type SoopDisplay = { [K in keyof SoopDisplayValues]: NonNullable<SoopDisplayValues[K]> };
export type SoopDisplayOverrides = Partial<SoopDisplay>;
// Legacy presentation, not a claim about unobserved historical preferences.
const fallback: SoopDisplay = { theme: 'dark', fontLevel: 3, randomNicknameColor: true,
  fanMessageColor: true, showEmoticons: true, animateEmoticons: true, smallOgq: false };
const fields = Object.keys(fallback) as (keyof SoopDisplay)[];
const domFields = new Set<keyof SoopDisplay>(['theme', 'fontLevel', 'randomNicknameColor']);
const paletteRoles = ['normal', 'streamer', 'manager', 'vip', 'subscribe', 'fan', 'support', 'nightbot'] as const;
function exactKeys(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
}
function paletteColor(value: unknown): value is string | null {
  return value === null || typeof value === 'string' && /^#[0-9a-f]{6}$/.test(value);
}
export function validSoopPalette(value: unknown): value is SoopDisplayPalette {
  if (!exactKeys(value, ['version', 'basis', 'nickname', 'roles']) || value.version !== 1
    || value.basis !== 'computed-css-variable' || !Array.isArray(value.nickname) || value.nickname.length !== 20
    || !exactKeys(value.roles, paletteRoles)) return false;
  const colors = value.nickname, roles = value.roles;
  return Array.from({ length: 20 }, (_, index) => colors[index]).every(paletteColor)
    && paletteRoles.every(key => paletteColor(roles[key]));
}
export function validSoopSurface(value: unknown): value is SoopDisplaySurface {
  return exactKeys(value, ['version', 'basis', 'foreground', 'background']) && value.version === 1
    && value.basis === 'computed-dom' && paletteColor(value.foreground) && paletteColor(value.background);
}
/** Preserve the official cascade; never replay arbitrary CSS from a sidecar. */
export function soopPaletteStyle(state?: SoopDisplayState | null, overrides: SoopDisplayOverrides = {}): CSSProperties | undefined {
  const theme = observedSoopDisplay(state).theme, palette = state?.palette;
  if (!theme || overrides.theme && overrides.theme !== theme) return undefined;
  const style: Record<string, string> = {};
  if (validSoopPalette(palette)) {
    palette.nickname.forEach((color, index) => { if (color !== null) style[`--nickname-ramdom-color${index + 1}`] = color; });
    for (const key of paletteRoles) if (palette.roles[key] !== null) style[`--${key}-color`] = palette.roles[key];
  }
  if (validSoopSurface(state?.surface)) {
    if (state.surface.foreground !== null) style['--soop-captured-foreground'] = state.surface.foreground;
    if (state.surface.background !== null) style.backgroundColor = state.surface.background;
  }
  return Object.keys(style).length ? style as CSSProperties : undefined;
}

function validValue(key: keyof SoopDisplay, value: unknown): boolean {
  if (key === 'theme') return value === 'light' || value === 'dark';
  if (key === 'fontLevel') return Number.isInteger(value) && Number(value) >= 1 && Number(value) <= 8;
  return typeof value === 'boolean';
}
export function observedSoopDisplay(state?: SoopDisplayState | null): SoopDisplayOverrides {
  const result: SoopDisplayOverrides = {};
  if (state?.version !== 1 || !state.values || !state.basis) return result;
  for (const key of fields) {
    const basis = state.basis[key];
    if (domFields.has(key) ? basis !== 'dom' : basis !== 'dom' && basis !== 'saved-option') continue;
    const value = state.values[key];
    if (validValue(key, value)) Object.assign(result, { [key]: value });
  }
  return result;
}
export function resolveSoopDisplay(state?: SoopDisplayState | null, overrides: SoopDisplayOverrides = {}): SoopDisplay {
  const result = { ...fallback, ...observedSoopDisplay(state) };
  for (const key of fields) if (validValue(key, overrides[key])) Object.assign(result, { [key]: overrides[key] });
  return result;
}
export function resolveSoopRowDisplay(row?: SoopRowDisplay, overrides: SoopDisplayOverrides = {}): Required<{ [K in keyof SoopRowDisplay]: boolean }> {
  const result = { fanMessageColor: fallback.fanMessageColor, showEmoticons: fallback.showEmoticons, smallOgq: fallback.smallOgq };
  for (const key of ['fanMessageColor', 'showEmoticons', 'smallOgq'] as const) {
    if (typeof row?.[key] === 'boolean') result[key] = row[key];
    if (typeof overrides[key] === 'boolean') result[key] = overrides[key];
  }
  return result;
}
export function soopDisplayLabel(state?: SoopDisplayState | null, overrides: SoopDisplayOverrides = {}, incomplete = false): string {
  if (fields.some(key => validValue(key, overrides[key]))) return '사용자 표시 설정';
  if (incomplete) return '당시 표시 설정 일부 누락';
  const known = Object.keys(observedSoopDisplay(state)).length;
  const label = known === fields.length ? '저장 당시 표시 설정' : known ? '저장 당시 표시 설정 일부' : '당시 표시 설정 없음';
  if (!known) return label;
  const colors = soopPaletteStyle(state);
  const count = colors ? Object.keys(colors).filter(key => key.startsWith('--nickname-ramdom-color') || paletteRoles.some(role => key === `--${role}-color`)).length : 0;
  const surface = validSoopSurface(state?.surface) ? state.surface : null;
  const surfaceCount = surface ? Number(surface.foreground !== null) + Number(surface.background !== null) : 0;
  return count === 28 && surfaceCount === 2 && !state?.paletteIncomplete && !state?.surfaceIncomplete ? label
    : `${label} · 색상 ${count || surfaceCount || state?.paletteIncomplete || state?.surfaceIncomplete ? '일부' : '기록 없음'}`;
}
