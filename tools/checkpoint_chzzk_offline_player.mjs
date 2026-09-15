// Explicit user-requested checkpoint. Never overwrites/restores/deletes anything.
import { mkdir, readdir, readFile, writeFile, lstat } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const directory = path.join(project, 'checkpoints', `chzzk-offline-player-${stamp}`);
await mkdir(path.dirname(directory), { recursive: true });
await mkdir(directory); // Refuse even a coincidentally existing checkpoint.
const entries = [];
async function preserve(relative, target = relative) {
  const from = path.join(project, relative), to = path.join(directory, target);
  const info = await lstat(from);
  if (info.isSymbolicLink()) throw new Error(`Checkpoint refuses links: ${relative}`);
  if (info.isDirectory()) {
    await mkdir(to, { recursive: true });
    for (const item of await readdir(from)) await preserve(path.join(relative, item), path.join(target, item));
    return;
  }
  if (!info.isFile()) throw new Error(`Not a regular file: ${relative}`);
  await mkdir(path.dirname(to), { recursive: true });
  const bytes = await readFile(from), hash = sha256(bytes);
  await writeFile(to, bytes, { flag: 'wx' });
  if (sha256(await readFile(to)) !== hash || sha256(await readFile(from)) !== hash) throw new Error(`Checkpoint verification failed: ${relative}`);
  entries.push({ file: target.replaceAll('\\', '/'), bytes: bytes.length, sha256: hash });
}
await preserve('.runtime/chzzk-offline-player', 'bundle');
for (const file of ['tools/chzzk-offline-player', 'tools/extract_chzzk_chat.mjs', 'public/original-player/source-manifest.json', 'package.json', 'pnpm-lock.yaml']) await preserve(file, path.join('source', file));
await preserve('.runtime/chzzk-offline-surface-inputs', 'rebuild-inputs/.runtime/chzzk-offline-surface-inputs');
for (const file of ['player-vendor-BYg0wCyN.js', 'common-vendor-pcHQV1G2.js', 'rolldown-runtime-CJJwijRH.js', 'player-vendor-Ct4cRQDi.css', 'vendor-DVDNLy6N.css', 'index-2Gtwn4qD.css', 'index-C4sif-4p.js', 'strings-ko_kr-CYa8lLdQ.js', 'icon_official_mark.png', 'synthetic.mp4']) {
  const relative = `.runtime/chzzk-original-player/${file}`;
  await preserve(relative, `rebuild-inputs/${relative}`);
}
await writeFile(path.join(directory, 'CHECKPOINT.json'), JSON.stringify({ format: 1, createdAt: new Date().toISOString(), purpose: 'Accepted offline player before Atsumi recording integration and VOD metadata overlay', verified: true, entries }, null, 2), { flag: 'wx' });
await writeFile(path.join(directory, 'README.md'), '# CHZZK 플레이어 체크포인트\n\n앱 녹화본 연결 직전 승인된 로컬 UI입니다. 소스/실행 자산의 SHA-256은 CHECKPOINT.json에 있습니다.\n\n- 독립 실행: 기존 18764 테스트 서버 종료 후 bundle/start.cmd 실행.\n- 소스 복구: source는 당시 소스, rebuild-inputs는 재생성용 원본 입력입니다. 현재 변경을 먼저 보존하고 필요한 파일만 비교하여 복원하세요. 이 묶음은 자동 복원/덮어쓰기를 하지 않습니다.\n- bundle에는 로컬 합성 영상만 있습니다. 사용자 녹화, 계정, 쿠키, DB는 포함하지 않습니다.\n- 이 폴더는 런타임 정리나 다음 빌드의 대상이 아닙니다.\n', { flag: 'wx' });
console.log(JSON.stringify({ checkpoint: directory, files: entries.length, bytes: entries.reduce((sum, item) => sum + item.bytes, 0), verified: true }));
