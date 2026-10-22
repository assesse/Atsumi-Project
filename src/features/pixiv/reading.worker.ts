// Private, on-demand worker. Only bundled dictionary URLs are fetched; no text
// or account information leaves this process. Termination releases the dictionary.
import DynamicDictionaries from "kuromoji/src/dict/DynamicDictionaries.js";
import Tokenizer from "kuromoji/src/Tokenizer.js";
import base from "kuromoji/dict/base.dat.gz?url";
import check from "kuromoji/dict/check.dat.gz?url";
import tid from "kuromoji/dict/tid.dat.gz?url";
import tidPos from "kuromoji/dict/tid_pos.dat.gz?url";
import tidMap from "kuromoji/dict/tid_map.dat.gz?url";
import cc from "kuromoji/dict/cc.dat.gz?url";
import unk from "kuromoji/dict/unk.dat.gz?url";
import unkPos from "kuromoji/dict/unk_pos.dat.gz?url";
import unkMap from "kuromoji/dict/unk_map.dat.gz?url";
import unkChar from "kuromoji/dict/unk_char.dat.gz?url";
import unkCompat from "kuromoji/dict/unk_compat.dat.gz?url";
import unkInvoke from "kuromoji/dict/unk_invoke.dat.gz?url";
import { hasKanji, romanizeTokens, safeMetadataText } from "./japaneseReading";

// Uncompressed sizes of the pinned 0.1.2 dictionary. The dev server may send
// Content-Encoding:gzip, in which case fetch has already decoded the bytes.
// Accept that case only for an exact known size; never guess on malformed data.
const decodedSizes = new Map<string, number>([
  [base, 8388608], [check, 8388608], [tid, 10485760], [tidPos, 41943040],
  [tidMap, 4194304], [cc, 3463716], [unk, 10485760], [unkPos, 10485760],
  [unkMap, 1048576], [unkChar, 65536], [unkCompat, 262144], [unkInvoke, 1048576],
]);

async function unpack(asset: string): Promise<ArrayBuffer> {
  const expectedLength = decodedSizes.get(asset);
  if (!expectedLength) throw new Error("unknown dictionary");
  const url = new URL(asset, globalThis.location.href);
  if (url.origin !== globalThis.location.origin && url.protocol !== "data:") throw new Error("dictionary origin");
  const response = await fetch(url, { credentials: "omit", signal: AbortSignal.timeout(10000) });
  if (!response.ok) throw new Error("dictionary unavailable");
  const compressed = new Uint8Array(await response.arrayBuffer());
  if (response.headers.get("Content-Encoding")?.toLowerCase() === "gzip" && !(compressed[0] === 0x1f && compressed[1] === 0x8b)) {
    if (compressed.length !== expectedLength) throw new Error("dictionary incomplete");
    return compressed.buffer;
  }
  if (compressed.length < 18 || compressed.length > 8 * 1024 * 1024) throw new Error("dictionary bounds");
  const length = new DataView(compressed.buffer).getUint32(compressed.length - 4, true);
  if (length !== expectedLength) throw new Error("dictionary bounds");
  const bytes = new Uint8Array(length);
  const reader = new Blob([compressed]).stream().pipeThrough(new DecompressionStream("gzip")).getReader();
  let offset = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (offset + value.length > length) throw new Error("dictionary bounds");
      bytes.set(value, offset); offset += value.length;
    }
    if (offset !== length) throw new Error("dictionary incomplete");
    return bytes.buffer;
  } finally { await reader.cancel().catch(() => {}); }
}
async function loadTokenizer() {
  const dictionary = new DynamicDictionaries();
  // Process groups serially so transient decompression buffers are bounded.
  const [b, c] = await Promise.all([unpack(base), unpack(check)]);
  dictionary.loadTrie(new Int32Array(b), new Int32Array(c));
  const [info, pos, map] = await Promise.all([unpack(tid), unpack(tidPos), unpack(tidMap)]);
  dictionary.loadTokenInfoDictionaries(new Uint8Array(info), new Uint8Array(pos), new Uint8Array(map));
  dictionary.loadConnectionCosts(new Int16Array(await unpack(cc)));
  const [u, p, m, ch, co, inv] = await Promise.all([unk, unkPos, unkMap, unkChar, unkCompat, unkInvoke].map(unpack));
  dictionary.loadUnknownDictionaries(new Uint8Array(u!), new Uint8Array(p!), new Uint8Array(m!), new Uint8Array(ch!), new Uint32Array(co!), new Uint8Array(inv!));
  return new Tokenizer(dictionary);
}
let tokenizer: Promise<Tokenizer> | undefined;
let latest = 0;
globalThis.onmessage = async (event: MessageEvent<unknown>) => {
  const request = event.data as { id?: unknown; text?: unknown } | null;
  if (!request || !Number.isSafeInteger(request.id) || !safeMetadataText(request.text) || !hasKanji(request.text)) return;
  const id = request.id as number, text = request.text; latest = id;
  try {
    tokenizer ??= loadTokenizer();
    const engine = await tokenizer;
    if (id !== latest) return;
    globalThis.postMessage({ id, result: romanizeTokens(engine.tokenize(text.normalize("NFKC"))) });
  } catch { globalThis.postMessage({ id, failed: true }); }
};
