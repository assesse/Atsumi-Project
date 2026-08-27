import jpFlag from "../assets/flags/bundled/jp.png";
import krFlag from "../assets/flags/bundled/kr.png";
import usFlag from "../assets/flags/bundled/us.png";
import cnFlag from "../assets/flags/cn.svg";
import type { Language } from "../core/types";

export type LanguagePresentation = {
  label: string;
  icon: string | null;
  fallback: string | null;
};

export const languageOrder: Language[] = ["korean", "japanese", "chinese", "english"];

export const languagePresentation: Record<Language, LanguagePresentation> = {
  korean: { label: "한국어", icon: krFlag, fallback: "KR" },
  japanese: { label: "일본어", icon: jpFlag, fallback: "JP" },
  chinese: { label: "중국어", icon: cnFlag, fallback: "CN" },
  english: { label: "영어", icon: usFlag, fallback: "US" },
};
