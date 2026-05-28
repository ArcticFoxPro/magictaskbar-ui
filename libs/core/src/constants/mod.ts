const _languageList = [
  { label: "English", enLabel: "English", value: "en" },
  { label: "中文 (简体)", enLabel: "Chinese (Simplified)", value: "zh-CN" },
] as const;

export type SupportedLanguagesCode = (typeof _languageList)[number]["value"];

export interface SupportedLanguage {
  label: string;
  enLabel: string;
  /** language code @example 'de' 'es' 'zh' 'en-US' 'en-UK' */
  value: string;
}

export const SupportedLanguages: SupportedLanguage[] = [..._languageList].sort((
  a,
  b,
) => a.label.localeCompare(b.label));
