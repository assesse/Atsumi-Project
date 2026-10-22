// Bounded local-only QA; owns its own hidden browser, server, fixture and profile.
// Never connects to an existing browser/app or sends metadata off this machine.
import { build, createServer } from "vite";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import http from "node:http";
import { spawn } from "node:child_process";
import assert from "node:assert/strict";

const development = false;
const root = process.cwd(), artifacts = path.join(root, ".runtime/x-official-20261018/browser", `${Date.now()}-${development ? "dev" : "production"}`), dist = path.join(artifacts, "dist");
await fs.mkdir(artifacts, { recursive: true });
const devServer = development ? await createServer({ cacheDir: path.join(artifacts, "dev-cache"), optimizeDeps: { entries: ["tools/x-session-preview.html"] }, server: { middlewareMode: true, hmr: false }, logLevel: "warn" }) : undefined;
if (!development) await build({ build: { outDir: dist, emptyOutDir: true, rollupOptions: { input: path.join(root, "tools/x-session-preview.html") } }, logLevel: "warn" });
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
const directory = await fs.mkdtemp(path.join(os.tmpdir(), "atsumi-x-session-qa-")), profile = path.join(directory, "profile");
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
  await page("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/tools/x-session-preview.html` });
  const waitFor=async(expression)=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await pause(70);}throw Error('Fixture condition failed: '+expression);};
  const capture=async(name)=>{const shot=await page("Page.captureScreenshot",{format:"png",captureBeyondViewport:false});await fs.writeFile(path.join(artifacts,name+'.png'),Buffer.from(shot.data,"base64"));report.scenes.push(name);};
  const click=async(label)=>{await evaluate(`(()=>{const button=[...document.querySelectorAll('button')].find(b=>b.textContent===${JSON.stringify(label)}||b.getAttribute('aria-label')===${JSON.stringify(label)});if(!button)throw Error('missing fixture button');button.click();})()`);await pause(100);};
  await waitFor("!!window.xFixture && document.body.textContent.includes('@studio_sample') && document.querySelector('.x-native-feed')?.getAttribute('aria-busy')==='false'");
  for(const [width,height] of [[1920,1080],[960,640],[320,640]]){
    await page("Emulation.setDeviceMetricsOverride",{width,height,deviceScaleFactor:1,mobile:false});
    await pause(100);
    await capture(width+'x'+height+'-empty');
    assert.equal(await evaluate("document.body.textContent.includes('↗')"),false);
    assert.equal(await evaluate("document.documentElement.scrollWidth>innerWidth+1"),false);
    await evaluate("window.xFixture.phase('loading')");
    await waitFor("!!document.querySelector('[aria-label=\"피드 불러오는 중\"]')");
    await capture(width+'x'+height+'-skeleton');
    await evaluate("window.xFixture.phase('ready');window.xFixture.preview('loading')");
    await waitFor("!!document.querySelector('[aria-label=\"게시물 불러오는 중\"]')");
    await capture(width+'x'+height+'-preview-loading');
    await evaluate("window.xFixture.preview('ready')");
    await waitFor("!!document.querySelector('.x-preview.has-selection')&&!document.querySelector('[aria-label=\"게시물 불러오는 중\"]')");
    await capture(width+'x'+height+'-native-slots');
    assert.equal(await evaluate("(()=>{const l=document.querySelector('.x-native-feed').getBoundingClientRect(),r=document.querySelector('.x-native-preview').getBoundingClientRect();return l.width>=200&&l.height>=80&&r.width>=100&&r.height>=80&&r.right<=innerWidth+1;})()"),true);
    await click('계정');
    await waitFor("!!window.xFixture.stats.lastViewport?.occlusions?.length");
    await capture(width+'x'+height+'-accounts');
    assert.equal(await evaluate("(()=>{const r=document.querySelector('[role=menu]').getBoundingClientRect();return r.left>=0&&r.top>=0&&r.right<=innerWidth+1&&r.bottom<=innerHeight+1;})()"),true);
    await click('@photo_sample');await waitFor("document.body.textContent.includes('@photo_sample')&&!document.querySelector('.x-preview.has-selection')&&document.querySelector('.x-native-feed')?.getAttribute('aria-busy')==='false'");
    await click('계정');await click('@studio_sample');await waitFor("document.body.textContent.includes('@studio_sample')&&document.querySelector('.x-native-feed')?.getAttribute('aria-busy')==='false'");
  }
  await page("Emulation.setDeviceMetricsOverride",{width:1920,height:1080,deviceScaleFactor:1,mobile:false});
  for(const label of ['팔로잉','가입된 X 커뮤니티','마음함','북마크']){
    await click(label);await waitFor("document.querySelector('.x-native-feed')?.getAttribute('aria-busy')==='false'");await capture('collection-'+label);
  }
  const beforeLogin=await evaluate("window.xFixture.stats.opens");
  await click('계정');await click('다시 로그인');
  await waitFor("document.body.textContent.includes('공식 X 창에서 로그인해 주세요.')");
  await waitFor("window.xFixture.stats.opens>"+beforeLogin+" && !document.body.textContent.includes('공식 X 창에서 로그인해 주세요.') && document.querySelector('.x-native-feed')?.getAttribute('aria-busy')==='false'");
  await capture('login-completion-return');
  await evaluate("window.xFixture.error('X 화면의 계정이 선택한 계정과 다릅니다. 계정 메뉴에서 다시 로그인해 주세요.')");
  await waitFor("!!document.querySelector('[role=alert]')");await capture('owner-mismatch');
  await evaluate("window.xFixture.privacy(true)");
  await waitFor("!document.querySelector('.x-native-feed')&&!document.querySelector('.x-preview')");
  await capture('privacy');
  report.stats=await evaluate("window.xFixture.stats");
  report.boundary="Offline production shell with synthetic native surface/account callbacks only. Empty native slots intentionally contain no X page. Does not verify real media, comments, videos, authenticated collections or WebView2.";
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
