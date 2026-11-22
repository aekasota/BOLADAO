import { create } from "zustand";
import { api, languageFromTag, LanguageSetting } from "../lib/ipc";
import { useSettingsStore } from "../stores/settingsStore";
import { en } from "./en";
import { es } from "./es";
import { Messages, pt } from "./pt";

export type Lang = "pt" | "en" | "es";
export type { Messages };

const CATALOG: Record<Lang, Messages> = { pt, en, es };

/** The supported languages, in the order the language chip lists them. */
export const LANGS: Lang[] = ["pt", "en", "es"];

/** Each language by its own name, as the language picker shows it. */
export const LANGUAGE_NAMES: Record<Lang, string> = { pt: "Português", en: "English", es: "Español" };

/** Value for `<html lang>`, read by screen readers. */
export const HTML_LANG: Record<Lang, string> = { pt: "pt-BR", en: "en", es: "es" };

const fallbackTag = typeof navigator !== "undefined" ? navigator.language : "pt-BR";

/** Windows interface language and regional format. Starts from the WebView and is confirmed by the backend. */
export const useSystemLanguage = create<{ language: Lang; locale: string }>(() => ({
  language: languageFromTag(fallbackTag),
  locale: fallbackTag,
}));

export async function loadSystemLanguage(): Promise<void> {
  try {
    useSystemLanguage.setState(await api.getSystemLanguage());
  } catch {
    // Keeps the WebView's language.
  }
}

export function resolveLang(setting: LanguageSetting | undefined, system: Lang): Lang {
  return !setting || setting === "auto" ? system : setting;
}

export function useLang(): Lang {
  const setting = useSettingsStore((s) => s.settings.language);
  const system = useSystemLanguage((s) => s.language);
  return resolveLang(setting, system);
}

export function useT(): Messages {
  return CATALOG[useLang()];
}

/** Current language outside of rendering (formatters, stores, event handlers). */
export function currentLang(): Lang {
  return resolveLang(
    useSettingsStore.getState().settings.language,
    useSystemLanguage.getState().language
  );
}

export function getT(): Messages {
  return CATALOG[currentLang()];
}

const decimalFormats = new Map<string, Intl.NumberFormat>();

/** Number with fixed decimals in the Windows regional format ("17,2" or "17.2"). */
export function formatDecimal(value: number, digits: number): string {
  const locale = useSystemLanguage.getState().locale || "pt-BR";
  const key = `${locale}|${digits}`;
  let fmt = decimalFormats.get(key);
  if (!fmt) {
    try {
      fmt = new Intl.NumberFormat(locale, {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
        useGrouping: false,
      });
    } catch {
      fmt = new Intl.NumberFormat("pt-BR", {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
        useGrouping: false,
      });
    }
    decimalFormats.set(key, fmt);
  }
  return fmt.format(value);
}

/** Date in the Windows regional format. */
export function formatDate(ms: number): string {
  const locale = useSystemLanguage.getState().locale;
  try {
    return new Date(ms).toLocaleDateString(locale || undefined);
  } catch {
    return new Date(ms).toLocaleDateString();
  }
}

const numberFormats = new Map<string, Intl.NumberFormat>();

/** Number with up to `maxDigits` decimals in the Windows regional format ("0,75" or "0.75"). */
export function formatNumber(value: number, maxDigits: number): string {
  const locale = useSystemLanguage.getState().locale || "pt-BR";
  const key = `${locale}|${maxDigits}`;
  let fmt = numberFormats.get(key);
  if (!fmt) {
    try {
      fmt = new Intl.NumberFormat(locale, { maximumFractionDigits: maxDigits, useGrouping: false });
    } catch {
      fmt = new Intl.NumberFormat("pt-BR", { maximumFractionDigits: maxDigits, useGrouping: false });
    }
    numberFormats.set(key, fmt);
  }
  return fmt.format(value);
}
