import type { ProcessingMode, SettingsSnapshot } from "../api/contracts";

export function isProcessingMode(value: unknown): value is ProcessingMode {
  return value === "basic" || value === "game_priority" || value === "atsumi_priority";
}

export function resolveProcessingMode(
  settings: Pick<SettingsSnapshot, "processingMode" | "highPerformanceProcessing">,
): ProcessingMode {
  if (isProcessingMode(settings.processingMode)) return settings.processingMode;
  return settings.highPerformanceProcessing === true ? "game_priority" : "basic";
}
