import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import type { Plugin } from "vite";

const require = createRequire(import.meta.url);
// Only bundled compressed reading data receives this local fetch prefix.
export const readingAssetName = (asset: { names: string[] }) => asset.names.some(name => /(?:^|\/)(?:base|check|cc|tid(?:_pos|_map)?|unk(?:_pos|_map|_char|_compat|_invoke)?)\.dat\.gz$/.test(name))
  ? "assets/pixiv-reading/[name]-[hash][extname]" : "assets/[name]-[hash][extname]";

export function pixivReadingNotices(): Plugin {
  return {
    name: "pixiv-offline-reading-notices",
    async generateBundle() {
      const kuromoji = dirname(require.resolve("kuromoji/package.json"));
      const localRequire = createRequire(join(kuromoji, "package.json"));
      const inputs = [
        ["kuromoji 0.1.2 — Apache-2.0", join(kuromoji, "LICENSE-2.0.txt")],
        ["IPADIC dictionary — upstream notices", join(kuromoji, "NOTICE.md")],
        ["doublearray — MIT", join(dirname(localRequire.resolve("doublearray/package.json")), "LICENSE.txt")],
        ["WanaKana 5.3.1 — MIT", join(dirname(require.resolve("wanakana/package.json")), "LICENSE")],
      ];
      const texts = await Promise.all(inputs.map(async ([heading, path]) => `${heading}\n\n${await readFile(path!, "utf8")}`));
      this.emitFile({ type: "asset", fileName: "assets/pixiv-reading/NOTICE.txt", source: texts.join("\n\n--------------------\n\n") });
    },
  };
}
