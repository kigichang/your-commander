import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { prettyChord } from "../keymap/keymap";
import { isMac } from "../lib/platform";
import en from "./locales/en.json";
import zhTW from "./locales/zh-TW.json";

/** Keys may carry `_mac` / `_linux` variants next to the base key; the
 *  matching variant replaces the base at init so call sites stay plain.
 *  A base key with no variant for this platform is used as written. */
function applyPlatformVariants(obj: Record<string, unknown>) {
  const suffix = isMac ? "_mac" : "_linux";
  for (const value of Object.values(obj)) {
    if (value && typeof value === "object") {
      applyPlatformVariants(value as Record<string, unknown>);
    }
  }
  for (const key of Object.keys(obj)) {
    if (key.endsWith("_mac") || key.endsWith("_linux")) {
      if (key.endsWith(suffix)) {
        obj[key.slice(0, key.lastIndexOf("_"))] = obj[key];
      }
      delete obj[key];
    }
  }
}

export function initI18n(saved: string | null) {
  const nav = navigator.language || "en";
  const lng = saved ?? (nav.startsWith("zh") ? "zh-TW" : "en");
  for (const bundle of [en, zhTW]) {
    applyPlatformVariants(bundle as unknown as Record<string, unknown>);
  }
  i18n.use(initReactI18next).init({
    resources: { en: { translation: en }, "zh-TW": { translation: zhTW } },
    lng,
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
      // Platform-aware shortcut names available to every string.
      defaultVariables: {
        mod: isMac ? "⌘" : "Ctrl",
        paste: prettyChord("mod+v"),
        settingsChord: prettyChord("mod+,"),
      },
    },
  });
  return i18n;
}

export default i18n;
