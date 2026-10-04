/**
 * localStorage under the `hermes.` namespace. Values saved before the rename
 * live under `gravity.` in the same WebView origin, so reads fall back to them
 * until the key is next written. Callers keep their own try/catch: storage can
 * be unavailable.
 */
const PREFIX = "hermes.";
const LEGACY_PREFIX = "gravity.";

export function readStored(name: string): string | null {
  return localStorage.getItem(PREFIX + name) ?? localStorage.getItem(LEGACY_PREFIX + name);
}

export function writeStored(name: string, value: string): void {
  localStorage.setItem(PREFIX + name, value);
}

/** Removes both spellings, so the legacy value cannot reappear as a fallback. */
export function removeStored(name: string): void {
  localStorage.removeItem(PREFIX + name);
  localStorage.removeItem(LEGACY_PREFIX + name);
}
