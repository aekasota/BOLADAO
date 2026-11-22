import React from "react";
import { Check, Languages } from "lucide-react";
import { HTML_LANG, Lang, LANGS, LANGUAGE_NAMES, useSystemLanguage, useT } from "../i18n";
import { useSettingsStore } from "../stores/settingsStore";
import { Popover, Toggle } from "../ui/Primitives";

/**
 * Status-bar chip with the interface language. It follows Windows until the user picks a language,
 * and picking one turns "follow Windows" off by itself.
 */
export const LanguageChip: React.FC = () => {
  const t = useT();
  const setting = useSettingsStore((s) => s.settings.language ?? "auto");
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const system = useSystemLanguage((s) => s.language);
  const follow = setting === "auto";
  const current: Lang = follow ? system : setting;

  const choose = (lang: Lang) => {
    if (follow && lang === system) return;
    updateSettings({ language: lang });
  };

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          type="button"
          className="font-mono-num status-chip"
          aria-label={t.app.language.chipAria(LANGUAGE_NAMES[current])}
        >
          <Languages size={12} aria-hidden="true" />
          {current.toUpperCase()}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content side="top" align="end" sideOffset={8} className="op-panel lang-pop">
          <div className="lang-pop-title">{t.app.language.title}</div>
          <div role="radiogroup" aria-label={t.app.language.title}>
            {LANGS.map((lang) => {
              const selected = current === lang;
              return (
                <button
                  key={lang}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  className="lang-pop-row"
                  data-selected={selected}
                  onClick={() => choose(lang)}
                >
                  <span className="font-mono-num lang-pop-code">{lang.toUpperCase()}</span>
                  <span lang={HTML_LANG[lang]} style={{ flex: 1 }}>
                    {LANGUAGE_NAMES[lang]}
                  </span>
                  {lang === system && <span className="lang-tag">Windows</span>}
                  <Check size={14} className="lang-pop-check" aria-hidden="true" />
                </button>
              );
            })}
          </div>
          <div className="lang-pop-foot">
            <Toggle
              checked={follow}
              onChange={(on) => updateSettings({ language: on ? "auto" : current })}
              label={t.app.language.followWindows}
            />
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
};
