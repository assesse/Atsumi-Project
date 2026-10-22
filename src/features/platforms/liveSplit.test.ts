import { describe, expect, it } from 'vitest';
import { clampLiveChat, liveSplit, liveSplitKey } from './liveSplit';

describe('individual live video/chat geometry', () => {
  it('uses the actual pair width instead of the application viewport', () => {
    expect(liveSplit(579, 300, 240, .44).compact).toBe(true);
    expect(liveSplit(580, 300, 240, .44).compact).toBe(false);
  });
  it('retains independent width and stacked proportion preferences', () => {
    expect(liveSplit(1000, 600, 180, .3).pixels).toBe(180);
    expect(liveSplit(400, 500, 180, .3).pixels).toBeCloseTo(493 * .3);
    expect(liveSplit(400, 800, 180, .3).pixels).toBeCloseTo(793 * .3);
    expect(liveSplit(1000, 600, 180, .3).pixels).toBe(180);
  });
  it('keeps wide-workspace paired cells horizontal without changing default single-stream layout', () => {
    const paired = liveSplit(475, 600, 180, .44, true);
    expect(paired.compact).toBe(false);
    expect(paired.available).toBe(468);
    expect(paired.pixels).toBe(180);
    expect(liveSplitKey('ArrowRight', paired.pixels, paired.min, paired.max, paired.compact)).toBe(160);
    expect(liveSplitKey('ArrowDown', paired.pixels, paired.min, paired.max, paired.compact)).toBeNull();
    expect(liveSplit(475, 600, 180, .44).compact).toBe(true);
  });
  it.each([[0, 0], [400, 5], [400, 160], [480, 305], [1920, 1080]])('fits finite dimensions even at %ix%i', (width, height) => {
    for (const value of [-1000, 0, 240, 99999, NaN, Infinity]) {
      const split = liveSplit(width, height, value, value);
      expect(split.pixels).toBeGreaterThanOrEqual(0);
      expect(split.pixels).toBeGreaterThanOrEqual(split.min);
      expect(split.pixels).toBeLessThanOrEqual(split.max);
      expect(split.pixels + split.min).toBeLessThanOrEqual(split.available + .001);
    }
  });
  it('allows single-stream chat below the previous160px minimum', () => {
    expect(liveSplit(1200, 700, 96, .44).pixels).toBe(96);
  });
  it('clamps keyboard and pointer requests to the same limits', () => {
    const split = liveSplit(480, 305, 240, .44);
    expect(liveSplitKey('Home', split.pixels, split.min, split.max, true)).toBe(split.min);
    expect(liveSplitKey('End', split.pixels, split.min, split.max, true)).toBe(split.max);
    expect(liveSplitKey('ArrowUp', split.pixels, split.min, split.max, true)).toBe(clampLiveChat(split.pixels + 20, split.min, split.max));
    expect(liveSplitKey('ArrowLeft', split.pixels, split.min, split.max, true)).toBeNull();
    expect(liveSplitKey('ArrowDown', split.min, split.min, split.max, true)).toBe(split.min);
    expect(liveSplitKey('ArrowLeft', 240, 96, 600, false)).toBe(260);
    expect(liveSplitKey('ArrowRight', 240, 96, 600, false)).toBe(220);
  });
});
