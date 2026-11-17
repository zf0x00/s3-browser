//! A Spacedrive-inspired palette: deep navy surfaces, floating panels, one
//! bright accent. The gpui-component theme is re-tinted to match, so dialogs,
//! inputs, menus and buttons belong to the same design.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Window background (behind the floating panels).
    pub app: Hsla,
    /// Explorer surface.
    pub surface: Hsla,
    /// Floating sidebar / inspector panels.
    pub panel: Hsla,
    pub panel_border: Hsla,
    /// Boxes inside panels (preview well, inputs).
    pub well: Hsla,
    pub line: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub ink: Hsla,
    pub ink_dull: Hsla,
    pub ink_faint: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_soft: Hsla,
    pub on_accent: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub backdrop: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub fn dark() -> Palette {
    Palette {
    dark: true,
    app: c(0x101116),
    surface: c(0x1c1d25),
    panel: c(0x15161d),
    panel_border: c(0x262733),
    well: c(0x101116),
    line: c(0x2a2b37),
    hover: c(0x24252f),
    selected: c(0x2c2e3b),
    ink: c(0xf2f3f7),
    ink_dull: c(0xa9acbd),
    ink_faint: c(0x7a7d8f),
    accent: c(0x2599ff),
    accent_hover: c(0x47a9ff),
    accent_soft: ca(0x2599ff2e),
    on_accent: c(0xffffff),
    success: c(0x34d399),
    danger: c(0xf87171),
    warning: c(0xfbbf24),
    backdrop: ca(0x07080ce6),
    }
}

pub fn light() -> Palette {
    Palette {
    dark: false,
    app: c(0xe9eaf0),
    surface: c(0xffffff),
    panel: c(0xf6f6f9),
    panel_border: c(0xe1e2ea),
    well: c(0xeceef3),
    line: c(0xe6e7ee),
    hover: c(0xeef0f5),
    selected: c(0xe3e7f2),
    ink: c(0x17181f),
    ink_dull: c(0x565a6c),
    ink_faint: c(0x8a8d9e),
    accent: c(0x1f8af0),
    accent_hover: c(0x3a9af5),
    accent_soft: ca(0x1f8af024),
    on_accent: c(0xffffff),
    success: c(0x059669),
    danger: c(0xdc2626),
    warning: c(0xd97706),
    backdrop: ca(0x0b0c12d9),
    }
}

pub fn palette(cx: &App) -> Palette {
    if cx.theme().is_dark() { dark() } else { light() }
}

/// Switch light/dark (or follow the system) and re-tint the components.
pub fn apply(dark: Option<bool>, window: Option<&mut Window>, cx: &mut App) {
    match dark {
        Some(true) => Theme::change(ThemeMode::Dark, window, cx),
        Some(false) => Theme::change(ThemeMode::Light, window, cx),
        None => Theme::sync_system_appearance(window, cx),
    }
    tint_components(cx);
}

fn tint_components(cx: &mut App) {
    let p = palette(cx);
    // `Theme::update` reconciles the component tokens (button backgrounds read
    // those) with the colours edited here; `global_mut` would skip that.
    Theme::update(cx, |theme| tint(theme, &p));
}

fn tint(theme: &mut Theme, p: &Palette) {
    let t = &mut theme.colors;
    t.background = p.surface;
    t.foreground = p.ink;
    t.border = p.line;
    t.input = p.line;
    t.ring = p.accent;
    t.caret = p.accent;
    t.selection = p.accent_soft;
    t.muted = p.well;
    t.muted_foreground = p.ink_faint;
    t.accent = p.hover;
    t.accent_foreground = p.ink;
    t.popover = p.panel;
    t.popover_foreground = p.ink;
    t.overlay = p.backdrop;
    t.primary = p.accent;
    t.primary_hover = p.accent_hover;
    t.primary_active = p.accent;
    t.primary_foreground = p.on_accent;
    t.button_primary = p.accent;
    t.button_primary_hover = p.accent_hover;
    t.button_primary_active = p.accent;
    t.button_primary_foreground = p.on_accent;
    t.secondary = p.hover;
    t.secondary_hover = p.selected;
    t.secondary_active = p.selected;
    t.secondary_foreground = p.ink;
    t.list_hover = p.hover;
    t.list_active = p.accent_soft;
    t.list_active_border = p.accent;
    t.sidebar = p.panel;
    t.sidebar_border = p.panel_border;
    t.sidebar_foreground = p.ink;
    t.sidebar_accent = p.hover;
    t.sidebar_accent_foreground = p.ink;
    t.title_bar = p.app;
    t.title_bar_border = p.app;
    t.window_border = p.panel_border;
    t.danger = p.danger;
    t.success = p.success;
    t.warning = p.warning;
    t.link = p.accent;
    t.progress_bar = p.accent;
    t.switch = p.selected;
    t.scrollbar = Hsla::transparent_black();
    t.scrollbar_thumb = p.selected;
    t.scrollbar_thumb_hover = p.ink_faint;
    t.tab_bar = p.panel;
    t.drop_target = p.accent_soft;
    t.drag_border = p.accent;
}
