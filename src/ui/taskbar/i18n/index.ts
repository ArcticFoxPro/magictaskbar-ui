import { SupportedLanguagesCode } from "@magic-ui/lib";
import i18n from "i18next";
import yaml from "js-yaml";
import { initReactI18next } from "react-i18next";

i18n.use(initReactI18next).init(
  {
    lng: "zh-CN",
    fallbackLng: "zh-CN",
    interpolation: {
      escapeValue: false,
    },
    resources: {},
  },
  undefined,
);

export async function loadTranslations() {
  const translations = {
    en: await import("./translations/en.yml"),
    "zh-CN": await import("./translations/zh-CN.yml"),
  };

  for (const [key, value] of Object.entries(translations)) {
    i18n.addResourceBundle(key, "translation", yaml.load(value.default));
  }
}

export default i18n;
