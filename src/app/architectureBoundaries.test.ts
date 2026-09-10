// @vitest-environment node
import { buildSync } from "esbuild";
import type { ComponentProps } from "react";
import { describe, expect, expectTypeOf, it } from "vitest";
import type { DanbooruApi } from "../api/featureClients";
import type { DanbooruWorkspace } from "../components/DanbooruWorkspace";
import appSource from "../App.tsx?raw";
import danbooruSource from "../components/DanbooruWorkspace.tsx?raw";
import hitomiSource from "../features/hitomi/HitomiFeature.tsx?raw";
import registrySource from "./workspaceRegistry.ts?raw";

const appModules = import.meta.glob<string>(["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"], {
  eager: true,
  query: "?raw",
  import: "default",
});

// Inspect module dependencies without executing the UI or backend. esbuild removes
// type-only imports and reports static imports, re-exports, and dynamic imports.
function runtimeImports(source: string): string[] {
  const result = buildSync({
    stdin: { contents: source, loader: "tsx" },
    bundle: false,
    write: false,
    metafile: true,
    format: "esm",
    logLevel: "silent",
    tsconfigRaw: { compilerOptions: { verbatimModuleSyntax: true } },
  });
  return Object.values(result.metafile.outputs).flatMap((output) => output.imports.map((dependency) => dependency.path));
}

function importsIncludingTypes(source: string): string[] {
  const declarations = [...source.matchAll(/^\s*(?:import|export)\s+(?:type\s+)?(?:[^;]*?\s+from\s+)?["']([^"']+)["']/gm)]
    .map((match) => match[1]!);
  const importTypes = [...source.matchAll(/\bimport\s*\(\s*["']([^"']+)["']/g)].map((match) => match[1]!);
  return [...declarations, ...importTypes, ...runtimeImports(source)];
}

const modulePath = (specifier: string): string => specifier.replace(/[?#].*$/, "").replace(/\.[cm]?[jt]sx?$/, "");
const isFeatureEntry = (specifier: string): boolean => /(?:^|\/)(?:HitomiFeature|DanbooruWorkspace)$/.test(modulePath(specifier));
const isBackendModule = (specifier: string): boolean => /(?:^|\/)api\/backend$/.test(modulePath(specifier));

describe("first-stage architecture boundaries", () => {
  it("keeps concrete workspace composition in App rather than shared app services", () => {
    expect(Object.keys(appModules)).toContain("./AppShell.tsx");
    for (const [filename, source] of Object.entries(appModules)) {
      expect(runtimeImports(source).filter(isFeatureEntry), `${filename} must not load concrete workspaces`).toEqual([]);
    }
    expect(runtimeImports(appSource).filter(isFeatureEntry).map(modulePath)).toEqual(expect.arrayContaining([
      "./components/DanbooruWorkspace",
      "./features/hitomi/HitomiFeature",
    ]));
  });

  it("keeps registry declarations independent of React, API transport, and gallery core types", () => {
    // Include static type imports/re-exports: registry purity applies to its type
    // dependencies as well as the dependencies retained in emitted JavaScript.
    expect(importsIncludingTypes(registrySource).filter((specifier) =>
      /^react(?:-dom)?(?:\/|$)/.test(specifier)
      || /(?:^|\/)(?:api|core)(?:\/|$)/.test(modulePath(specifier)),
    )).toEqual([]);
  });

  it("keeps Hitomi from directly importing the Danbooru workspace", () => {
    expect(importsIncludingTypes(hitomiSource).filter((specifier) => /(?:^|\/)DanbooruWorkspace$/.test(modulePath(specifier)))).toEqual([]);
    // Legacy ActivityDrawer/SettingsDialog integration intentionally remains in
    // HitomiFeature; this guard does not claim those models are source-neutral.
  });

  it("keeps Danbooru on its injected narrow API without loading the backend singleton", () => {
    expect(runtimeImports(danbooruSource).filter(isBackendModule)).toEqual([]);
    expectTypeOf<ComponentProps<typeof DanbooruWorkspace>["backend"]>().toEqualTypeOf<DanbooruApi>();
  });
});
