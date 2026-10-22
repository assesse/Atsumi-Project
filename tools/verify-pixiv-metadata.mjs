// Bounded local-only QA; owns its own hidden browser, server, fixture and profile.
// Never connects to an existing browser/app or sends metadata off this machine.
import { build, createServer } from "vite";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import http from "node:http";
import { spawn } from "node:child_process";
import assert from "node:assert/strict";

const development = process.argv.includes("--dev");
const root = process.cwd(), artifacts = path.join(root, ".runtime/pixiv-metadata-reading/browser", `${Date.now()}-${development ? "dev" : "production"}`), dist = path.join(artifacts, "dist");
await fs.mkdir(artifacts, { recursive: true });
const devServer = development ? await createServer({ cacheDir: path.join(artifacts, "dev-cache"), optimizeDeps: { entries: ["tools/pixiv-metadata-preview.html"] }, server: { middlewareMode: true, hmr: false }, logLevel: "warn" }) : undefined;
if (!development) await build({ build: { outDir: dist, emptyOutDir: true, rollupOptions: { input: path.join(root, "tools/pixiv-metadata-preview.html") } }, logLevel: "warn" });
const requests = [], report = { development, scenes: [], requests, errors: [] };
const server = http.createServer(async (req, res) => {
  try {
    const pathname = decodeURIComponent(new URL(req.url, "http://127.0.0.1").pathname);
    if (devServer) { requests.push(pathname); devServer.middlewares(req, res, () => res.writeHead(404).end()); return; }
    const file = path.resolve(dist, "." + pathname), relative = path.relative(dist, file);
    if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) { res.writeHead(404).end(); return; }
    const bytes = await fs.readFile(file); requests.push(pathname);
    const type = pathname.endsWith(".js") ? "text/javascript" : pathname.endsWith(".css") ? "text/css" : pathname.endsWith(".html") ? "text/html; charset=utf-8" : "application/octet-stream";
    res.setHeader("Content-Type", type); res.setHeader("Cache-Control", "no-store");
    res.setHeader("Content-Security-Policy", `default-src 'none'; script-src 'self'; worker-src 'self'; connect-src http://127.0.0.1:${server.address().port}/assets/pixiv-reading/; style-src 'self' 'unsafe-inline'; img-src data:`);
    res.end(bytes);
  } catch { res.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const directory = await fs.mkdtemp(path.join(os.tmpdir(), "atsumi-pixiv-reading-qa-")), profile = path.join(directory, "profile");
const browser = process.env.ATSUMI_TEST_BROWSER || "C:/Program Files/Google/Chrome/Application/chrome.exe";
const child = spawn(browser, ["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "about:blank"], { windowsHide: true, stdio: "ignore" });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const pending = new Map(); let socket, send, page;
try {
  let port, endpoint;
  for (let i = 0; i < 100; i++) { try { [port, endpoint] = (await fs.readFile(path.join(profile, "DevToolsActivePort"), "utf8")).trim().split(/\r?\n/); if (port && endpoint) break; } catch {} if (child.exitCode !== null) throw Error("QA browser failed"); await pause(100); }
  assert.match(port ?? "", /^\d+$/); socket = new WebSocket(`ws://127.0.0.1:${port}${endpoint}`);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let id = 0;
  socket.onmessage = event => { const message = JSON.parse(event.data), call = pending.get(message.id); if (!call) return; clearTimeout(call.timer); pending.delete(message.id); if (message.error) call.reject(Error(JSON.stringify(message.error))); else call.resolve(message.result); };
  send = (method, params = {}, sessionId) => new Promise((resolve, reject) => { const next = ++id, timer = setTimeout(() => { pending.delete(next); reject(Error(`QA timeout: ${method}`)); }, 22000); pending.set(next, { resolve, reject, timer }); socket.send(JSON.stringify({ id: next, method, params, ...(sessionId ? { sessionId } : {}) })); });
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  page = (method, params = {}) => send(method, params, sessionId);
  const evaluate = async expression => { const result = await page("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }); if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails)); return result.result.value; };
  await page("Page.enable");
  await page("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/tools/pixiv-metadata-preview.html` });
  for (let i = 0; i < 100; i++) { if (await evaluate("!!document.querySelector('[data-fixture-ready]')")) break; await pause(100); }
  assert.equal(await evaluate("!!document.querySelector('[data-fixture-ready]')"), true);
  report.dictionaryFetchesBeforeHover = requests.filter(p => p.endsWith(".gz")).length; assert.equal(report.dictionaryFetchesBeforeHover, 0);
  for (const [width, height] of [[1920, 1080], [320, 640]]) {
    await page("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
    for (const name of ["tag", "kana", "artist", "group", "title", "unknown", "long", "corner"]) {
      // Fixture-only selectors. Never enumerate or evaluate a user page.
      const point = await evaluate(`(()=>{const el=document.getElementById(${JSON.stringify(name)});el.scrollIntoView({block:'center'});const r=el.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
      await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: point.x, y: point.y }); await pause(450);
      const started = Date.now();
      const result = await evaluate(`(async()=>{for(let i=0;i<160;i++){const el=document.querySelector('[role="tooltip"]');if(el&&!el.textContent.includes('발음 확인 중')){const r=el.getBoundingClientRect();return {text:el.textContent,left:r.left,top:r.top,right:r.right,bottom:r.bottom,clientHeight:el.clientHeight,scrollHeight:el.scrollHeight};}await new Promise(r=>setTimeout(r,100));}throw Error('tooltip not ready');})()`);
      const scene = { name, width, height, waitMs: Date.now() - started, ...result }; report.scenes.push(scene);
      const screenshot = await page("Page.captureScreenshot", { format: "png", captureBeyondViewport: false }); await fs.writeFile(path.join(artifacts, `${width}x${height}-${name}.png`), Buffer.from(screenshot.data, "base64"));
      assert.ok(result.left >= 0 && result.top >= 0 && result.right <= width && result.bottom <= height, `${name}: tooltip outside viewport`);
      assert.ok(result.scrollHeight <= result.clientHeight + 1, `${name}: tooltip content clipped`);
      if (name === "tag") assert.ok(result.text.includes("풍경") && result.text.includes("landscape"));
      if (name === "kana") assert.ok(result.text.includes("midori"));
      if (name === "artist") assert.ok(/영어 발음yamada.+\(추정\)/i.test(result.text), `worker failed: ${result.text}`);
      assert.ok(!/Pixiv 발음|로마자 발음|추정 발음|한국어 표기|원문 표기/.test(result.text), `unexpected heading: ${result.text}`);
      if (name === "corner") assert.ok(result.text.includes("풍경"));
      if (name === "kana" && width === 1920) assert.equal(requests.filter(p => p.endsWith(".gz")).length, 0);
      await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: 1, y: 1 }); await pause(40);
    }
  }
  await evaluate("document.getElementById('tag').dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:true}))"); await pause(40);
  assert.equal(await evaluate("document.getElementById('click-result').textContent"), "風景:true");
  report.dictionaryFiles = [...new Set(requests.filter(p => p.endsWith(".gz")))].length; assert.equal(report.dictionaryFiles, 12);
  report.workerTargets = (await send("Target.getTargets")).targetInfos.filter(t => t.type === "worker").length;
  await pause(20500);
  report.workerTargetsAfterIdle = (await send("Target.getTargets")).targetInfos.filter(t => t.type === "worker").length;
  assert.equal(report.workerTargetsAfterIdle, 0);
  report.passed = true;
} catch (error) {
  report.errors.push(String(error)); report.passed = false;
  if (development) {
    report.assetDiagnostics = [];
    for (const pathname of [...new Set(requests.filter(p => p.endsWith(".gz")))].slice(0, 2)) {
      const response = await fetch(`http://127.0.0.1:${server.address().port}${pathname}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      report.assetDiagnostics.push({ pathname, status: response.status, headers: Object.fromEntries(response.headers), size: bytes.length, firstBytes: [...bytes.slice(0, 16)] });
    }
  }
  if (page) { try { const screenshot = await page("Page.captureScreenshot", { format: "png" }); await fs.writeFile(path.join(artifacts, "failure.png"), Buffer.from(screenshot.data, "base64")); } catch {} }
  process.exitCode = 1;
} finally {
  await fs.writeFile(path.join(artifacts, "verification.json"), JSON.stringify(report, null, 2));
  if (socket?.readyState === WebSocket.OPEN) await send?.("Browser.close").catch(() => {});
  for (const call of pending.values()) clearTimeout(call.timer); socket?.close();
  server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  await devServer?.close();
  // Preserve the isolated profile as evidence; never force-close another process.
  console.log(JSON.stringify({ passed: report.passed, scenes: report.scenes.length, errors: report.errors, artifacts, profile }));
}
