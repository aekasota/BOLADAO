//! Interface language. Portuguese is the original; English and Spanish are written in a neutral
//! register so they read naturally in any country. The backend keeps the current language in a
//! global so messages built anywhere (errors, job stages, the tray, notifications) follow it.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Pt,
    En,
    Es,
}

/// What the user picked in the settings: follow Windows, or a fixed language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LanguageSetting {
    #[default]
    Auto,
    Pt,
    En,
    Es,
}

#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);

// Tests run in parallel threads: each one gets its own language so a test that switches it cannot
// change the messages another test is checking.
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(0) };
}

fn load() -> u8 {
    #[cfg(not(test))]
    {
        CURRENT.load(Ordering::Relaxed)
    }
    #[cfg(test)]
    {
        CURRENT.with(|c| c.load(Ordering::Relaxed))
    }
}

fn store(code: u8) {
    #[cfg(not(test))]
    CURRENT.store(code, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(code, Ordering::Relaxed));
}

pub fn current() -> Lang {
    match load() {
        1 => Lang::En,
        2 => Lang::Es,
        _ => Lang::Pt,
    }
}

pub fn set(lang: Lang) {
    store(match lang {
        Lang::Pt => 0,
        Lang::En => 1,
        Lang::Es => 2,
    });
}

pub fn resolve(setting: LanguageSetting) -> Lang {
    match setting {
        LanguageSetting::Auto => system(),
        LanguageSetting::Pt => Lang::Pt,
        LanguageSetting::En => Lang::En,
        LanguageSetting::Es => Lang::Es,
    }
}

pub fn apply(setting: LanguageSetting) {
    set(resolve(setting));
}

/// Maps a BCP 47 tag ("pt-BR", "es-419", "en-US") to a supported language. Anything that is not
/// Portuguese or Spanish gets English, the most widely read of the three.
pub fn from_tag(tag: &str) -> Lang {
    let tag = tag.trim().to_ascii_lowercase();
    if tag.starts_with("pt") {
        Lang::Pt
    } else if tag.starts_with("es") {
        Lang::Es
    } else {
        Lang::En
    }
}

/// Language of the Windows interface.
pub fn system() -> Lang {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        // Primary language identifiers from winnt.h.
        const LANG_PORTUGUESE: u16 = 0x16;
        const LANG_SPANISH: u16 = 0x0a;
        // SAFETY: plain query with no arguments.
        let primary = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
        match primary {
            LANG_PORTUGUESE => Lang::Pt,
            LANG_SPANISH => Lang::Es,
            _ => Lang::En,
        }
    }
    #[cfg(not(windows))]
    {
        from_tag(&std::env::var("LANG").unwrap_or_default())
    }
}

/// Regional format chosen in Windows ("pt-BR", "es-MX"), used for numbers and dates.
pub fn system_locale() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        // SAFETY: the buffer has LOCALE_NAME_MAX_LENGTH (85) characters, as the API requires.
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if len > 1 {
            return String::from_utf16_lossy(&buf[..(len as usize - 1)]);
        }
        "pt-BR".to_string()
    }
    #[cfg(not(windows))]
    {
        "pt-BR".to_string()
    }
}

/// Picks the message for the current language. Each argument is a `format!` string, so named
/// arguments are taken from the surrounding scope: `tr!("Erro em {path}", "Error in {path}", ...)`.
#[macro_export]
macro_rules! tr {
    ($pt:literal, $en:literal, $es:literal $(,)?) => {
        match $crate::i18n::current() {
            $crate::i18n::Lang::Pt => format!($pt),
            $crate::i18n::Lang::En => format!($en),
            $crate::i18n::Lang::Es => format!($es),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_system_tags_to_supported_languages() {
        assert_eq!(from_tag("pt-BR"), Lang::Pt);
        assert_eq!(from_tag("pt-PT"), Lang::Pt);
        assert_eq!(from_tag("es-419"), Lang::Es);
        assert_eq!(from_tag("ES-mx"), Lang::Es);
        assert_eq!(from_tag("en-GB"), Lang::En);
        assert_eq!(from_tag("fr-FR"), Lang::En);
        assert_eq!(from_tag(""), Lang::En);
    }

    #[test]
    fn messages_follow_the_current_language() {
        let file = "a.mp4";
        let pick = || tr!("Arquivo {file} pronto", "File {file} ready", "Archivo {file} listo");
        set(Lang::En);
        assert_eq!(pick(), "File a.mp4 ready");
        set(Lang::Es);
        assert_eq!(pick(), "Archivo a.mp4 listo");
        set(Lang::Pt);
        assert_eq!(pick(), "Arquivo a.mp4 pronto");
    }
}
