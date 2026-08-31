use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Language {
    #[default]
    Chinese,
    English,
}

impl Language {
    pub(crate) const ALL: [Self; 2] = [Self::Chinese, Self::English];

    pub(crate) fn database_value(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en-US",
        }
    }

    pub(crate) fn from_database(value: &str) -> Self {
        match value {
            "en" | "en-US" | "en_US" => Self::English,
            _ => Self::Chinese,
        }
    }

    pub(crate) fn select_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or_default()
    }

    pub(crate) fn native_name(self) -> &'static str {
        match self {
            Self::Chinese => "简体中文",
            Self::English => "English",
        }
    }
}

static CURRENT_LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn current_language() -> Language {
    match CURRENT_LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::English,
        _ => Language::Chinese,
    }
}

pub(crate) fn set_language(language: Language) {
    let value = match language {
        Language::Chinese => 0,
        Language::English => 1,
    };
    CURRENT_LANGUAGE.store(value, Ordering::Relaxed);
}

pub(crate) fn localized(chinese: &'static str, english: &'static str) -> &'static str {
    match current_language() {
        Language::Chinese => chinese,
        Language::English => english,
    }
}

/// Selects localized static copy while keeping both locales beside the same semantic use.
///
/// Persistence and window state remain centralized here, so additional resource strategies
/// can be introduced without coupling UI call sites to the settings store.
#[macro_export]
macro_rules! tr {
    ($chinese:expr, $english:expr $(,)?) => {
        $crate::i18n::localized($chinese, $english)
    };
}

/// Formats a complete localized message. Both branches own their sentence structure so
/// translations never have to be assembled from localized fragments.
#[macro_export]
macro_rules! tr_args {
    ($chinese:literal, $english:literal $(, $arguments:expr)* $(,)?) => {
        match $crate::i18n::current_language() {
            $crate::i18n::Language::Chinese => format!($chinese $(, $arguments)*),
            $crate::i18n::Language::English => format!($english $(, $arguments)*),
        }
    };
}
