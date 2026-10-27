import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zhTW from "./locales/zh-TW.json";

export function initI18n(saved: string | null) {
  const nav = navigator.language || "en";
  const lng = saved ?? (nav.startsWith("zh") ? "zh-TW" : "en");
  i18n.use(initReactI18next).init({
    resources: { en: { translation: en }, "zh-TW": { translation: zhTW } },
    lng,
    fallbackLng: "en",
    interpolation: { escapeValue: false },
  });
  return i18n;
}

export default i18n;
