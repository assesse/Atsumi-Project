import { describe, expect, it } from "vitest";
import styles from "../styles.css?raw";

// jsdom cannot paint pseudo-elements. Inspect the actual stylesheet contract;
// browser QA separately checks mask opacity throughout hover and reduced motion.
const stylesheet = styles.replace(/\/\*[\s\S]*?\*\//g, "");
const ruleBodies = (selector: string): string[] => [...stylesheet.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
  .filter((match) => match[1]!.split(/,\s*(?![^()]*\))/).some((value) => value.trim() === selector))
  .map((match) => match[2]!);

describe("artist folder thumbnail privacy layers", () => {
  const single = '.download-artist-folder-preview-stack[data-work-count="1"] .download-artist-folder-preview';
  const overlay = ".download-artist-folder-overlay-preview";

  it.each([single, overlay])("keeps %s decoration off the privacy pseudo-element", (selector) => {
    expect(ruleBodies(`${selector}::before`).join("\n")).toContain("z-index: 1");
    expect(ruleBodies(`${selector}::before`).join("\n")).toContain("display: none");
    expect(ruleBodies(`${selector}::after`)).toEqual([]);
  });

  it("animates only the decorative layer on hover, focus, and expanded previews", () => {
    const sheenRules = [...stylesheet.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter((match) => /animation:\s*download-artist-folder-cover-sheen/.test(match[2]!));
    expect(sheenRules).toHaveLength(2);
    for (const rule of sheenRules) {
      expect(rule[1]).toContain("::before");
      expect(rule[1]).not.toContain("::after");
    }
    expect(sheenRules[1]![1]).toContain(":hover");
    expect(sheenRules[1]![1]).toContain(":focus-visible");
  });

  it("keeps the shared privacy mask above decoration without fading or moving", () => {
    const mask = ruleBodies('[data-privacy-mode="on"] .gallery-thumbnail::after');
    expect(mask).toHaveLength(1);
    expect(mask[0]).toContain("z-index: 3");
    expect(mask[0]).toContain("backdrop-filter: blur(28px)");
    expect(mask[0]).not.toMatch(/(?:opacity|animation|transform|display):/);
  });
});
