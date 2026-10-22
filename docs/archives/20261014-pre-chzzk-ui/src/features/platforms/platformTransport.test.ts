// @vitest-environment node
import config from '../../../src-tauri/tauri.conf.json';
import { expect, it } from 'vitest';

it('allows HLS script requests only to the revocable local live transport', () => {
  const csp = config.app.security.csp as Record<string, string>;
  expect(csp['connect-src']?.split(/\s+/).sort()).toEqual([
    'ipc:', 'http://ipc.localhost', 'platform-media://localhost/live/',
    'http://platform-media.localhost/live/', 'https://platform-media.localhost/live/',
  ].sort());
  expect(csp['script-src']).toBe("'self'");
  expect(csp['object-src']).toBe("'none'");
  expect(csp['worker-src']).toBe("'self' blob:");
  expect(csp['connect-src']).not.toMatch(/\*|https:\s|youtube|sooplive|googlevideo|127\.0\.0\.1/);
});
