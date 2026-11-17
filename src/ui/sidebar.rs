//! The floating sidebar: connection switcher, buckets, library, footer.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, FontWeight, SharedString, div, img, px};

use super::{GAP, SIDEBAR_W, caption, drag_area, icon};
use crate::app::S3App;
use crate::cache;
use crate::connection_form::provider_badge;
use crate::theme::Palette;
use crate::util;

fn nav_row(id: impl Into<SharedString>, selected: bool, p: &Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    h_flex()
        .id(gpui_kit::ElementId::Name(id.into()))
        .h(px(30.))
        .px(px(8.))
        .gap(px(9.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(13.))
        .when(selected, |d| d.bg(p.selected).text_color(p.ink))
        .when(!selected, |d| d.text_color(p.ink_dull).hover(|s| s.bg(p.hover).text_color(p.ink)))
}

impl S3App {
    pub(super) fn render_sidebar(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.as_ref();
        let active_id = session.map(|s| s.conn.id.clone());

        // ---- connection switcher --------------------------------------------
        let current = session.map(|s| s.conn.clone());
        let connecting = self.connecting.is_some();
        let switcher = h_flex()
            .id("switcher")
            .h(px(36.))
            .px(px(8.))
            .gap(px(8.))
            .rounded(px(9.))
            .bg(p.well)
            .border_1()
            .border_color(p.line)
            .cursor_pointer()
            .hover(|s| s.border_color(p.ink_faint))
            .map(|d| match &current {
                Some(conn) => d
                    .child(div().size(px(7.)).rounded_full().bg(p.success))
                    .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(conn.name.clone())),
                None => d
                    .child(div().size(px(7.)).rounded_full().bg(if connecting { p.warning } else { p.ink_faint }))
                    .child(
                        div()
                            .flex_1()
                            .text_color(p.ink_dull)
                            .child(if connecting { "Connecting…" } else { "Choose connection" }),
                    ),
            })
            .child(icon(IconName::ChevronsUpDown, 14., p.ink_faint))
            .on_click(cx.listener(|this, _, _, cx| {
                this.switcher_open = !this.switcher_open;
                cx.notify();
            }));

        let show_list = self.switcher_open || session.is_none();
        let connections = show_list.then(|| {
            let rows = self.connections.iter().map(|conn| {
                let id = conn.id.clone();
                let is_active = active_id.as_deref() == Some(id.as_str());
                let is_connecting = self.connecting.as_deref() == Some(id.as_str());
                let host = if conn.endpoint.is_empty() {
                    format!("AWS · {}", conn.region)
                } else {
                    conn.endpoint.trim_start_matches("https://").trim_start_matches("http://").to_string()
                };
                let entity = cx.entity();
                let (connect_id, edit_conn, dup_id, del_id) = (id.clone(), conn.clone(), id.clone(), id.clone());
                h_flex()
                    .id(SharedString::from(format!("conn-{id}")))
                    .px(px(6.))
                    .py(px(5.))
                    .gap(px(9.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(is_active, |d| d.bg(p.selected))
                    .hover(|s| s.bg(p.hover))
                    .child(provider_badge(conn.provider, 24.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).truncate().child(conn.name.clone()))
                            .child(div().text_size(px(11.)).text_color(p.ink_faint).truncate().child(host)),
                    )
                    .when(is_connecting, |d| d.child(icon(IconName::LoaderCircle, 13., p.ink_faint)))
                    .when(is_active, |d| d.child(icon(IconName::Check, 13., p.accent)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.session.as_ref().map(|s| s.conn.id.as_str()) == Some(connect_id.as_str()) {
                            this.switcher_open = false;
                            cx.notify();
                        } else {
                            this.connect(&connect_id, window, cx);
                        }
                    }))
                    .context_menu(move |menu, _, _| {
                        let (e1, e2, e3) = (entity.clone(), entity.clone(), entity.clone());
                        let (edit_conn, dup_id, del_id) = (edit_conn.clone(), dup_id.clone(), del_id.clone());
                        menu.item(PopupMenuItem::new("Edit…").icon(IconName::Pencil).on_click(move |_, window, cx| {
                            let conn = edit_conn.clone();
                            e1.update(cx, |this, cx| this.open_connection_dialog(Some(conn), None, window, cx))
                        }))
                        .item(PopupMenuItem::new("Duplicate").icon(IconName::Copy).on_click(move |_, window, cx| {
                            e2.update(cx, |this, cx| this.duplicate_connection(&dup_id, window, cx))
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Remove").icon(IconName::Trash).on_click(move |_, window, cx| {
                            let id = del_id.clone();
                            e3.update(cx, |this, cx| this.delete_connection(id, window, cx))
                        }))
                    })
            });
            v_flex()
                .p(px(4.))
                .gap(px(2.))
                .rounded(px(10.))
                .bg(p.well)
                .border_1()
                .border_color(p.line)
                .children(rows)
                .child(
                    h_flex()
                        .id("new-conn")
                        .px(px(8.))
                        .py(px(7.))
                        .gap(px(8.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(p.accent)
                        .hover(|s| s.bg(p.hover))
                        .child(icon(IconName::Plus, 14., p.accent))
                        .child("New connection…")
                        .on_click(cx.listener(|this, _, window, cx| this.open_connection_dialog(None, None, window, cx))),
                )
                .when_some(session.map(|s| s.conn.clone()), |d, conn| {
                    d.child(
                        h_flex()
                            .id("disconnect")
                            .px(px(8.))
                            .py(px(7.))
                            .gap(px(8.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .text_color(p.ink_dull)
                            .hover(|s| s.bg(p.hover))
                            .child(icon(IconName::Unplug, 14., p.ink_faint))
                            .child(format!("Disconnect {}", conn.name))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switcher_open = false;
                                this.disconnect(cx);
                            })),
                    )
                })
        });

        // ---- buckets -----------------------------------------------------------
        let buckets = session.map(|session| {
            let current = session.bucket.clone();
            let rows = session.buckets.iter().map(|b| {
                let name = b.name.clone();
                let selected = current.as_deref() == Some(name.as_str());
                let (open, del) = (name.clone(), name.clone());
                let entity = cx.entity();
                nav_row(format!("bucket-{name}"), selected, p)
                    .child(img("art/bucket.svg").size(px(16.)))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_bucket(open.clone(), window, cx)))
                    .context_menu(move |menu, _, _| {
                        let (e, name) = (entity.clone(), del.clone());
                        menu.item(PopupMenuItem::new("Delete bucket…").icon(IconName::Trash).on_click(move |_, window, cx| {
                            let n = name.clone();
                            e.update(cx, |this, cx| this.delete_bucket(n, window, cx))
                        }))
                    })
            });
            v_flex()
                .gap(px(1.))
                .child(
                    h_flex()
                        .px(px(8.))
                        .pt(px(16.))
                        .pb(px(6.))
                        .child(caption("Buckets", p))
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("refresh-buckets")
                                .cursor_pointer()
                                .child(icon(
                                    if session.buckets_loading { IconName::LoaderCircle } else { IconName::RefreshCw },
                                    12.,
                                    p.ink_faint,
                                ))
                                .on_click(cx.listener(|this, _, window, cx| this.refresh_buckets(window, cx))),
                        )
                        .child(
                            div()
                                .id("new-bucket")
                                .ml(px(8.))
                                .cursor_pointer()
                                .child(icon(IconName::Plus, 13., p.ink_faint))
                                .on_click(cx.listener(|this, _, window, cx| this.create_bucket(window, cx))),
                        ),
                )
                .when(session.buckets.is_empty() && !session.buckets_loading, |d| {
                    d.child(div().px(px(8.)).text_size(px(12.)).text_color(p.ink_faint).child("No buckets"))
                })
                .children(rows)
        });

        // ---- library ----------------------------------------------------------
        let overview_selected = session.is_some_and(|s| s.bucket.is_none());
        let active_transfers = self.transfers.iter().filter(|t| t.is_active()).count();
        let library = session.is_some().then(|| {
            v_flex()
                .gap(px(1.))
                .child(div().px(px(8.)).pt(px(16.)).pb(px(6.)).child(caption("Library", p)))
                .child(
                    nav_row("overview", overview_selected, p)
                        .child(icon(IconName::Boxes, 15., p.accent))
                        .child("Overview")
                        .on_click(cx.listener(|this, _, window, cx| this.open_bucket_or_home(None, window, cx))),
                )
                .child(
                    nav_row("transfers", self.show_transfers, p)
                        .child(icon(IconName::ArrowUpDown, 15., p.accent))
                        .child(div().flex_1().child("Transfers"))
                        .when(active_transfers > 0, |d| {
                            d.child(
                                div()
                                    .px(px(6.))
                                    .rounded_full()
                                    .bg(p.accent)
                                    .text_color(p.on_accent)
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::BOLD)
                                    .child(active_transfers.to_string()),
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_transfers = !this.show_transfers;
                            cx.notify();
                        })),
                )
                .child(
                    nav_row("cache", false, p)
                        .child(icon(IconName::HardDrive, 15., p.accent))
                        .child("Cache & previews")
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                )
        });

        // ---- footer: egress saved, settings, theme ----------------------------
        let stats = cache::global().stats();
        let footer = v_flex()
            .gap(px(8.))
            .p(px(8.))
            .border_t_1()
            .border_color(p.panel_border)
            .child(
                h_flex()
                    .id("saved")
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .bg(p.accent_soft)
                    .cursor_pointer()
                    .child(icon(IconName::PiggyBank, 15., p.accent))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if stats.saved == 0 {
                                        "No repeat downloads yet".to_string()
                                    } else {
                                        format!("{} not re-downloaded", util::human_size(stats.saved))
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(p.ink_faint)
                                    .child(format!("{} fetched from storage", util::human_size(stats.fetched))),
                            ),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
            .child(
                h_flex()
                    .gap(px(2.))
                    .child(
                        super::tool_button("sb-settings", IconName::Settings, "Cache & previews")
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                    )
                    .child(
                        super::tool_button(
                            "sb-theme",
                            if p.dark { IconName::Sun } else { IconName::Moon },
                            "Toggle light / dark",
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        super::tool_button("sb-collapse", IconName::PanelLeftClose, "Hide sidebar").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.settings.sidebar = false;
                                this.save_settings(cx);
                            },
                        )),
                    ),
            );

        div()
            .p(px(GAP))
            .pr(px(0.))
            .h_full()
            .flex_shrink_0()
            .child(
                v_flex()
                    .w(px(SIDEBAR_W))
                    .h_full()
                    .rounded(px(12.))
                    .bg(p.panel)
                    .border_1()
                    .border_color(p.panel_border)
                    .overflow_hidden()
                    // Room for the traffic lights; also drags the window.
                    .child(drag_area("sidebar-drag").h(px(34.)).flex_shrink_0())
                    .child(
                        v_flex()
                            .id("sidebar-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(8.))
                            .pb(px(8.))
                            .gap(px(6.))
                            .child(switcher)
                            .children(connections)
                            .children(buckets)
                            .children(library),
                    )
                    .child(footer),
            )
            .into_any_element()
    }
}
