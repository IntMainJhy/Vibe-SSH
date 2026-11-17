import { useEffect, useState } from "react";
export type Theme = "system" | "light" | "dark";
function stored(key: string) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
export function usePreferences() {
  const [theme, setTheme] = useState<Theme>(() => {
    const value = stored("vibe.theme");
    return value === "light" || value === "dark" ? value : "system";
  });
  const [fontSize, setFontSize] = useState(() =>
    Math.max(11, Math.min(20, Number(stored("vibe.font")) || 13)),
  );
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("vibe.theme", theme);
    } catch {
      /* Storage can be unavailable. */
    }
  }, [theme]);
  useEffect(() => {
    try {
      localStorage.setItem("vibe.font", String(fontSize));
    } catch {
      /* Keep session preference. */
    }
  }, [fontSize]);
  return { theme, setTheme, fontSize, setFontSize };
}
