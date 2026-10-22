import { describe, expect, it, vi } from 'vitest';
import { subscribeCardProfile } from './currentProfile';
import type { PlatformMediaApi } from './platformMedia';
const image = 'data:image/png;base64,AQ==';
const api = (profile: unknown, current: unknown = image) => ({ profile: vi.fn().mockResolvedValue(profile), currentProfile: vi.fn().mockResolvedValue(current) }) as unknown as PlatformMediaApi;
const settle = async () => { for (let n=0;n<30;n++) await Promise.resolve(); };
describe('library card portrait only', () => {
  it('prefers immutable proof and explicitly marks a current-picture fallback', async () => {
    const local = api(image), now = api(null), a=vi.fn(), b=vi.fn();
    subscribeCardProfile(local,'a',a); subscribeCardProfile(now,'b',b); await settle();
    expect(local.currentProfile).not.toHaveBeenCalled(); expect(a).toHaveBeenCalledWith({image,current:false});
    expect(now.currentProfile).toHaveBeenCalledWith('b'); expect(b).toHaveBeenCalledWith({image,current:true});
  });
  it('does not hide invalid or corrupt historical proof by fetching a current portrait', async () => {
    for (const current of [api('https://example.invalid/image'), api(null)]) {
      if (current.profile && (await current.profile('a')) === null) vi.mocked(current.profile).mockRejectedValue(new Error('corrupt'));
      const done=vi.fn(); subscribeCardProfile(current,'a',done); await settle();
      expect(current.currentProfile).not.toHaveBeenCalled(); expect(done).toHaveBeenCalledWith(null);
    }
  });
  it('cancels a pending public fallback after the card disappears', async () => {
    let resolve!: (value:string|null)=>void; const current=api(null), done=vi.fn(); current.profile=vi.fn(()=>new Promise<string|null>(r=>{resolve=r;}));
    const release=subscribeCardProfile(current,'a',done); release(); resolve(null); await settle();
    expect(current.currentProfile).not.toHaveBeenCalled(); expect(done).not.toHaveBeenCalled();
  });
  it('bounds reads to two and removes unsubscribed queued work', async () => {
    const resolvers: ((value:string|null)=>void)[]=[]; const current=api(null); current.profile=vi.fn(()=>new Promise<string|null>(r=>resolvers.push(r)));
    const releases=Array.from({length:8},(_,n)=>subscribeCardProfile(current,String(n),vi.fn()));
    expect(current.profile).toHaveBeenCalledTimes(2); releases.forEach(release=>release()); resolvers.forEach(resolve=>resolve(null)); await settle();
    expect(current.profile).toHaveBeenCalledTimes(2); expect(current.currentProfile).not.toHaveBeenCalled();
  });
});
