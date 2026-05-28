use std::sync::OnceLock;

use crate::state::theme::ThemeId;

pub struct BuiltinDefaultTheme {
    pub shared_css: &'static str,
    pub taskbar_css: &'static str,
}

static BUILTIN_DEFAULT_THEME: OnceLock<BuiltinDefaultTheme> = OnceLock::new();

pub fn set_builtin_default_theme(theme: BuiltinDefaultTheme) {
    let _ = BUILTIN_DEFAULT_THEME.set(theme);
}

pub fn get_builtin_default_theme() -> Option<&'static BuiltinDefaultTheme> {
    BUILTIN_DEFAULT_THEME.get()
}

pub fn get_builtin_taskbar_css() -> Option<&'static str> {
    BUILTIN_DEFAULT_THEME.get().map(|t| t.taskbar_css)
}

pub fn get_builtin_shared_css() -> Option<&'static str> {
    BUILTIN_DEFAULT_THEME.get().map(|t| t.shared_css)
}

pub fn is_default_theme_id(id: &ThemeId) -> bool {
    id.as_str() == "@default/theme"
}
