export type TagChipMeasurement = {
  width: number;
  height: number;
};

export type TagFitResult = {
  visibleCount: number;
  hiddenCount: number;
  showOverflow: boolean;
};

export type DisplayTag = {
  value: string;
  favorite: boolean;
  namespace: "female" | "male" | "neutral";
};

const tagNamespace = (tag: string): DisplayTag["namespace"] => {
  const namespace = tag.slice(0, tag.indexOf(":"));
  return namespace === "female" || namespace === "male" ? namespace : "neutral";
};

export function sortGalleryTags(
  tags: readonly string[],
  favoriteMetadata: ReadonlySet<string>,
): DisplayTag[] {
  const namespaceOrder: Record<DisplayTag["namespace"], number> = {
    female: 0,
    male: 1,
    neutral: 2,
  };
  return tags
    .map((value, originalIndex) => ({
      value,
      favorite: favoriteMetadata.has(value),
      namespace: tagNamespace(value),
      originalIndex,
    }))
    .sort((left, right) =>
      Number(right.favorite) - Number(left.favorite)
      || namespaceOrder[left.namespace] - namespaceOrder[right.namespace]
      || left.originalIndex - right.originalIndex)
    .map(({ originalIndex: _originalIndex, ...tag }) => tag);
}

function chipsFit(
  chips: readonly TagChipMeasurement[],
  availableWidth: number,
  availableHeight: number,
  gapX: number,
  gapY: number,
): boolean {
  if (!chips.length) return true;
  if (availableWidth <= 0 || availableHeight <= 0) return false;

  let rowWidth = 0;
  let rowHeight = 0;
  let usedHeight = 0;

  for (const chip of chips) {
    const width = Math.min(availableWidth, Math.max(0, chip.width));
    const height = Math.max(0, chip.height);
    if (!width || !height) return false;

    const startsNewRow = rowWidth > 0 && rowWidth + gapX + width > availableWidth;
    if (startsNewRow) {
      usedHeight += rowHeight + gapY;
      rowWidth = 0;
      rowHeight = 0;
    }

    if (usedHeight + height > availableHeight) return false;
    rowWidth = rowWidth > 0 ? rowWidth + gapX + width : width;
    rowHeight = Math.max(rowHeight, height);
  }

  return true;
}

/**
 * Fits measured tag chips into a fixed card rectangle. Overflow chip
 * measurements are indexed by digit count and use tabular numerals in CSS, so
 * +9, +10 and +100 reserve their real rendered widths without guessing fonts.
 */
export function fitTagChips(
  chips: readonly TagChipMeasurement[],
  overflowByDigits: readonly TagChipMeasurement[],
  availableWidth: number,
  availableHeight: number,
  gapX: number,
  gapY: number,
): TagFitResult {
  if (!chips.length) return { visibleCount: 0, hiddenCount: 0, showOverflow: false };
  if (chipsFit(chips, availableWidth, availableHeight, gapX, gapY)) {
    return { visibleCount: chips.length, hiddenCount: 0, showOverflow: false };
  }

  for (let visibleCount = chips.length - 1; visibleCount >= 0; visibleCount -= 1) {
    const hiddenCount = chips.length - visibleCount;
    const overflow = overflowByDigits[String(hiddenCount).length - 1];
    if (!overflow) continue;
    if (chipsFit(
      [...chips.slice(0, visibleCount), overflow],
      availableWidth,
      availableHeight,
      gapX,
      gapY,
    )) {
      return { visibleCount, hiddenCount, showOverflow: true };
    }
  }

  return { visibleCount: 0, hiddenCount: chips.length, showOverflow: false };
}

export function splitGalleryTitle(title: string, subtitle?: string): {
  primary: string;
  secondary: string;
} {
  const canonical = title.trim();
  const pipeIndex = canonical.indexOf("|");
  const primary = (pipeIndex >= 0 ? canonical.slice(0, pipeIndex) : canonical).trim();
  const pipedSecondary = pipeIndex >= 0
    ? canonical.slice(pipeIndex + 1).split("|").map((part) => part.trim()).filter(Boolean)
    : [];
  const explicitSecondary = subtitle?.trim() ?? "";
  const safePrimary = primary || canonical;
  const secondaryParts = [...pipedSecondary, explicitSecondary]
    .filter((part, index, parts) => Boolean(part) && part !== safePrimary && parts.indexOf(part) === index);
  return {
    primary: safePrimary,
    secondary: secondaryParts.join(" · "),
  };
}
