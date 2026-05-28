import { FuncCommand } from "@magic-ui/lib";
import { ResourceText } from "@magic-ui/lib/types";
import { invoke } from "@tauri-apps/api/core";

export function getRootContainer(): HTMLElement {
  const element = document.getElementById("root");
  if (!element) {
    throw new Error("Root element not found");
  }
  return element;
}

export function toPhysicalPixels(size: number): number {
  return Math.round(size * globalThis.devicePixelRatio);
}

export async function applyTextScaleCompensation(): Promise<void> {
  try {
    const textScale = await invoke<number>("get_text_scale_factor");
    const rootStyle = document.documentElement.style as CSSStyleDeclaration & { zoom: string };
    rootStyle.zoom = textScale && textScale > 1.0 ? `${(1 / textScale) * 100}%` : "";
  } catch (e) {
    console.warn("[TextScale] Failed to apply compensation", e);
  }
}

export function wasInstalledUsingMSIX(): Promise<boolean> {
  return invoke(FuncCommand.IsAppxPackage);
}

export function isDev(): Promise<boolean> {
  return invoke(FuncCommand.IsDevMode);
}

export function getResourceText(text: ResourceText, locale: string): string {
  if (typeof text === "string") {
    return text;
  }
  return text[locale] || text["en"] || "Unknown";
}

// Difference between Windows epoch (1601) and Unix epoch (1970) in milliseconds
const EPOCH_DIFF_MILLISECONDS = 11644473600000n;

/** Convert Windows FileTime to Js Unix Date */
export function WindowsDateFileTimeToDate(fileTime: bigint | number) {
  if (typeof fileTime === "number") fileTime = BigInt(fileTime);
  return new Date(Number(fileTime / 10000n - EPOCH_DIFF_MILLISECONDS));
}
