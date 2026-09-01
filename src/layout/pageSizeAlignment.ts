const positiveInteger = (value: number, fallback: number): number => (
  Number.isFinite(value) && value >= 1 ? Math.max(1, Math.trunc(value)) : fallback
);

/**
 * Keeps result pages rectangular for the currently measured card grid.
 * The configured value is treated as a target: round up to the next full row,
 * or use the largest full row below the source limit when rounding up would
 * exceed it.
 */
export function alignPageSizeToColumns(
  configuredSize: number,
  columns: number,
  maximumSize: number,
): number {
  const maximum = positiveInteger(maximumSize, 1);
  const safeColumns = Math.min(maximum, positiveInteger(columns, 1));
  const target = Math.min(maximum, positiveInteger(configuredSize, safeColumns));
  const roundedUp = Math.ceil(target / safeColumns) * safeColumns;
  if (roundedUp <= maximum) return roundedUp;
  return Math.max(safeColumns, Math.floor(maximum / safeColumns) * safeColumns);
}
