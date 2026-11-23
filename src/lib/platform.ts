// The system the app runs on. The backend says it in `window.__BOLADAO_OS__` (see lib.rs); the
// user agent is the fallback (browser preview).

export type Platform = "windows" | "macos" | "linux";

function detect(): Platform {
  const injected = (window as unknown as { __BOLADAO_OS__?: string }).__BOLADAO_OS__;
  if (injected === "windows" || injected === "macos" || injected === "linux") return injected;
  const ua = navigator.userAgent;
  if (/Macintosh|Mac OS X/i.test(ua)) return "macos";
  if (/Linux|X11/i.test(ua)) return "linux";
  return "windows";
}

export const PLATFORM: Platform = detect();

/** The system's own name, as people know it. */
export const OS_NAME: string = { windows: "Windows", macos: "macOS", linux: "Linux" }[PLATFORM];

/** Shortcut modifier as the system shows it: ⌘ on macOS, Ctrl elsewhere. */
export const MOD_KEY: string = PLATFORM === "macos" ? "⌘" : "Ctrl";

/** Picks the text (or value) written for this system. */
export function forPlatform<T>(variants: Record<Platform, T>): T {
  return variants[PLATFORM];
}
