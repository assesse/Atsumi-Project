// Browser-preview policy only. The native protocol replaces this module with
// an allowlist from Tauri config; a web request cannot choose its trusted port.
const url = new URL(import.meta.url);
export const previewOrigin = url.protocol === 'http:' && url.hostname === '127.0.0.1'
  && !url.username && !url.password ? url.origin : null;
export const parentOrigins = ['tauri://localhost', 'http://tauri.localhost', 'https://tauri.localhost',
  ...(previewOrigin ? [previewOrigin] : [])];
