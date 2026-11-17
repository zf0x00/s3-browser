//! Rendering. The layout follows Spacedrive's explorer:
//!
//! ```text
//! ┌────────────┬──────────────────────────────────────┬──────────────┐
//! │ (traffic)  │ ‹ › [📁 photos]   🔍 search   ▦ ≡ ▣ ⓘ │  preview     │
//! │ ● Conn  ⌄  │                                      │  name        │
//! │ BUCKETS    │   thumbnails grid / list / media     │  DETAILS     │
//! │  media     │                                      │  DATES       │
//! │  backups   │                                      │  STORAGE     │
//! │ LIBRARY    ├──────────────────────────────────────┤              │
//! │ ⚙  ☾  ⇅    │ bucket › photos › 2024     12 items  │  actions     │
//! └────────────┴──────────────────────────────────────┴──────────────┘
//! ```

mod explorer;
mod inspector;
mod overlays;
mod sidebar;

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, FontWeight, Hsla, MouseButton, ObjectFit, Stateful, Window,
    WindowControlArea, div, img, px, rgb,
};

use crate::app::S3App;
use crate::menus;
use crate::settings::ViewMode;
use crate::s3::Entry;
use crate::theme::{self, Palette};
use crate::util;

pub const SIDEBAR_W: f32 = 232.;
pub const INSPECTOR_W: f32 = 284.;
pub const GAP: f32 = 8.;
pub const TOPBAR_H: f32 = 52.;

/// An empty area that drags the window (our content reaches the top edge).
pub fn drag_area(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

pub fn icon(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// Small ghost icon button with a tooltip.
pub fn tool_button(id: impl Into<ElementId>, name: IconName, tooltip: &'static str) -> Button {
    Button::new(id).ghost().small().icon(name).tooltip(tooltip)
}

/// Section caption (BUCKETS, DETAILS …).
pub fn caption(text: &str, p: &Palette) -> Div {
    div()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.ink_faint)
        .child(text.to_uppercase())
}

/// The artwork for an entry: thumbnail, folder or file with an extension badge.
/// `size` is the square box it must fit; `cover` fills it (media view).
pub fn file_visual(entry: &Entry, thumb: Option<PathBuf>, size: f32, cover: bool, p: &Palette) -> AnyElement {
    if entry.is_dir {
        return div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(img("art/folder.svg").w(px(size * 0.86)).h(px(size * 0.69)))
            .into_any_element();
    }
    if let Some(path) = thumb {
        let image = img(path)
            .object_fit(if cover { ObjectFit::Cover } else { ObjectFit::Contain })
            .rounded(px(if cover { 4. } else { (size * 0.05).clamp(3., 8.) }));
        return div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(if cover { image.size_full() } else { image.size(px(size * 0.92)) })
            .into_any_element();
    }
    file_art(&entry.name, size, p)
}

/// A document glyph with a coloured extension badge (Spacedrive style).
pub fn file_art(name: &str, size: f32, p: &Palette) -> AnyElement {
    let art = if p.dark { "art/file-dark.svg" } else { "art/file-light.svg" };
    let (w, h) = (size * 0.62, size * 0.78);
    let badge = util::badge(name).filter(|_| size >= 40.).map(|(label, color)| {
        div()
            .absolute()
            .bottom(px(h * 0.16))
            .left(px(-size * 0.06))
            .px(px((size * 0.045).max(3.)))
            .py(px(1.))
            .rounded(px((size * 0.04).max(3.)))
            .bg(rgb(color))
            .text_color(gpui_kit::white())
            .text_size(px((size * 0.11).clamp(8., 13.)))
            .font_weight(FontWeight::BOLD)
            .child(label)
    });
    div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .relative()
                .w(px(w))
                .h(px(h))
                .child(img(art).size_full())
                .when(util::is_video(name) && size >= 40., |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(size * 0.26))
                                    .rounded_full()
                                    .bg(rgb(0xec4899))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Play, size * 0.13, gpui_kit::white())),
                            ),
                    )
                })
                .children(badge),
        )
        .into_any_element()
}

/// A rounded segmented control (view modes).
pub fn segmented(items: Vec<(IconName, &'static str, bool, Box<dyn Fn(&mut Window, &mut gpui_kit::App)>)>, p: &Palette) -> Div {
    let mut row = h_flex().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(p.well).border_1().border_color(p.line);
    for (i, (name, tip, active, on_click)) in items.into_iter().enumerate() {
        row = row.child(
            div()
                .id(("seg", i))
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(active, |d| d.bg(p.selected))
                .when(!active, |d| d.hover(|s| s.bg(p.hover)))
                .child(icon(name, 15., if active { p.ink } else { p.ink_faint }))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx))
                .on_click(move |_, window, cx| on_click(window, cx)),
        );
    }
    row
}

impl Render for S3App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::palette(cx);
        let in_bucket = self.session.as_ref().is_some_and(|s| s.bucket.is_some());
        let show_sidebar = self.settings.sidebar;
        let show_inspector = self.settings.inspector && in_bucket;

        let sidebar = show_sidebar.then(|| self.render_sidebar(&p, cx));
        let topbar = self.render_topbar(&p, window, cx);
        let content = self.render_content(&p, window, cx);
        let pathbar = in_bucket.then(|| self.render_pathbar(&p, cx));
        let inspector = show_inspector.then(|| self.render_inspector(&p, cx));
        let transfers = self.show_transfers.then(|| self.render_transfers(&p, cx));
        let preview = self.preview.is_some().then(|| self.render_preview(&p, window, cx));

        div()
            .id("root")
            .size_full()
            .relative()
            .bg(p.surface)
            .text_color(p.ink)
            .font_family(cx.theme().font_family.clone())
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event, window, cx| this.on_key(event, window, cx)))
            .on_action(cx.listener(|this, _: &menus::About, window, cx| this.show_about(window, cx)))
            .on_action(cx.listener(|this, _: &menus::OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|this, _: &menus::NewConnection, window, cx| this.open_connection_dialog(None, None, window, cx)))
            .on_action(cx.listener(|this, _: &menus::UploadFiles, window, cx| this.upload_dialog(false, window, cx)))
            .on_action(cx.listener(|this, _: &menus::UploadFolder, window, cx| this.upload_dialog(true, window, cx)))
            .on_action(cx.listener(|this, _: &menus::NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(cx.listener(|_, _: &menus::CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &menus::Refresh, window, cx| this.reload(window, cx)))
            .on_action(cx.listener(|this, _: &menus::GoBack, window, cx| this.go_back(window, cx)))
            .on_action(cx.listener(|this, _: &menus::GoForward, window, cx| this.go_forward(window, cx)))
            .on_action(cx.listener(|this, _: &menus::GoUp, window, cx| this.go_up(window, cx)))
            .on_action(cx.listener(|this, _: &menus::ViewGrid, _, cx| this.set_view(ViewMode::Grid, cx)))
            .on_action(cx.listener(|this, _: &menus::ViewList, _, cx| this.set_view(ViewMode::List, cx)))
            .on_action(cx.listener(|this, _: &menus::ViewMedia, _, cx| this.set_view(ViewMode::Media, cx)))
            .on_action(cx.listener(|this, _: &menus::ZoomIn, _, cx| this.zoom(1, cx)))
            .on_action(cx.listener(|this, _: &menus::ZoomOut, _, cx| this.zoom(-1, cx)))
            .on_action(cx.listener(|this, _: &menus::ToggleSidebar, _, cx| {
                this.settings.sidebar = !this.settings.sidebar;
                this.save_settings(cx);
            }))
            .on_action(cx.listener(|this, _: &menus::ToggleInspector, _, cx| {
                this.settings.inspector = !this.settings.inspector;
                this.save_settings(cx);
            }))
            .on_action(cx.listener(|this, _: &menus::ShowTransfers, _, cx| {
                this.show_transfers = !this.show_transfers;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &menus::ToggleTheme, window, cx| this.toggle_theme(window, cx)))
            .on_action(cx.listener(|_, _: &menus::Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &menus::ZoomWindow, window, _| window.zoom_window()))
            .child(
                h_flex()
                    .size_full()
                    .children(sidebar)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(topbar)
                            .child(div().flex_1().min_h_0().child(content))
                            .children(pathbar),
                    )
                    .children(inspector),
            )
            .children(transfers)
            .children(preview)
    }
}

/// Placeholder used by several empty states.
pub fn empty_state(icon_name: IconName, title: &str, body: &str, p: &Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .p(px(24.))
        .child(
            div()
                .size(px(64.))
                .rounded(px(18.))
                .bg(p.well)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icon_name, 28., p.ink_faint)),
        )
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .when(!body.is_empty(), |d| {
            d.child(div().max_w(px(420.)).text_center().text_size(px(12.5)).text_color(p.ink_faint).child(body.to_string()))
        })
        .into_any_element()
}
