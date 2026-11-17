//! The floating inspector: large preview, name, and DETAILS / DATES /
//! STORAGE / METADATA sections with actions at the bottom.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, div, img, px};

use super::{GAP, INSPECTOR_W, file_visual, icon, tool_button};
use crate::app::{Detail, FolderSize, S3App};
use crate::theme::Palette;
use crate::util;

fn section(icon_name: IconName, title: &str, p: &Palette) -> Div {
    v_flex().gap(px(7.)).child(
        h_flex()
            .gap(px(7.))
            .pb(px(2.))
            .child(icon(icon_name, 13., p.accent))
            .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.ink_dull).child(title.to_uppercase())),
    )
}

fn row(label: &str, value: impl Into<String>, p: &Palette) -> Div {
    h_flex()
        .gap(px(12.))
        .text_size(px(12.))
        .child(div().flex_shrink_0().text_color(p.ink_faint).child(label.to_string()))
        .child(div().flex_1().min_w_0().flex().justify_end().child(div().truncate().child(value.into())))
}

fn divider(p: &Palette) -> Div {
    div().h(px(1.)).w_full().bg(p.line)
}

impl S3App {
    pub(super) fn render_inspector(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.selected_entries(cx);
        let session = self.session.as_ref().unwrap();
        let bucket = session.bucket.clone().unwrap_or_default();
        let well = INSPECTOR_W - 32.;

        let mut body = v_flex().gap(px(16.));
        let mut actions = h_flex().gap(px(2.)).justify_center();

        match entries.as_slice() {
            [] => {
                let name = if session.prefix.is_empty() {
                    bucket.clone()
                } else {
                    session.prefix.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
                };
                let total: u64 = session.entries.iter().map(|e| e.size).sum();
                let files = session.entries.iter().filter(|e| !e.is_dir).count();
                body = body
                    .child(preview_well(img(if session.prefix.is_empty() { "art/bucket.svg" } else { "art/folder.svg" }).size(px(well * 0.5)).into_any_element(), well, p))
                    .child(title_block(name, if session.prefix.is_empty() { "Bucket".into() } else { "Folder".into() }, p))
                    .child(divider(p))
                    .child(
                        section(IconName::Info, "Details", p)
                            .child(row("Items", session.entries.len().to_string(), p))
                            .child(row("Files", files.to_string(), p))
                            .child(row("Size", util::human_size(total), p)),
                    )
                    .child(
                        section(IconName::HardDrive, "Storage", p)
                            .child(row("Location", format!("s3://{bucket}/{}", session.prefix), p))
                            .child(row("Connection", session.conn.name.clone(), p))
                            .child(row("Provider", session.conn.provider.info().name, p)),
                    );
                actions = actions
                    .child(tool_button("insp-upload", IconName::Upload, "Upload files").on_click(cx.listener(|this, _, window, cx| this.upload_dialog(false, window, cx))))
                    .child(tool_button("insp-newfolder", IconName::FolderPlus, "New folder").on_click(cx.listener(|this, _, window, cx| this.new_folder(window, cx))));
            }
            [entry] if entry.is_dir => {
                let key = entry.key.clone();
                let open_key = key.clone();
                let count_key = key.clone();
                let (size_text, files_text, can_count_all) = match self.folder_size(&key) {
                    Some(FolderSize::Done(s)) if s.complete => (util::human_size(s.bytes), util::group_digits(s.files), false),
                    Some(FolderSize::Done(s)) => (
                        format!("more than {}", util::human_size(s.bytes)),
                        format!("{}+", util::group_digits(s.files)),
                        true,
                    ),
                    Some(FolderSize::Failed) => ("Couldn't calculate".into(), "—".into(), true),
                    _ => ("Calculating…".into(), "Calculating…".into(), false),
                };
                body = body
                    .child(preview_well(file_visual(entry, None, well * 0.62, false, p), well, p))
                    .child(title_block(entry.name.clone(), "Folder".into(), p))
                    .child(divider(p))
                    .child(
                        section(IconName::Info, "Details", p)
                            .child(row("Size", size_text, p))
                            .child(row("Files", files_text, p))
                            .when(can_count_all, |d| {
                                d.child(
                                    Button::new("insp-count-all")
                                        .outline()
                                        .xsmall()
                                        .icon(IconName::Gauge)
                                        .label("Count everything")
                                        .on_click(cx.listener(move |this, _, _, cx| this.calc_folder_size(count_key.clone(), true, cx))),
                                )
                            }),
                    )
                    .child(
                        section(IconName::HardDrive, "Storage", p)
                            .child(row("Bucket", bucket.clone(), p))
                            .child(row("Prefix", key.clone(), p))
                            .child(row("URI", format!("s3://{bucket}/{key}"), p)),
                    );
                actions = actions
                    .child(tool_button("insp-open", IconName::FolderOpen, "Open").on_click(cx.listener(move |this, _, window, cx| this.navigate(open_key.clone(), window, cx))))
                    .child(tool_button("insp-dl", IconName::Download, "Download folder…").on_click(cx.listener(|this, _, window, cx| this.download_selected(window, cx))))
                    .child(tool_button("insp-rename", IconName::Pencil, "Rename…").on_click(cx.listener(|this, _, window, cx| this.rename_selected(window, cx))))
                    .child(tool_button("insp-del", IconName::Trash, "Delete…").on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))));
            }
            [entry] => {
                let thumb = self.thumb_for(entry);
                let cached = self.is_cached(entry);
                let ext = util::extension_of(&entry.name);
                let meta = match &self.detail {
                    Some(Detail::Ready(m)) if m.key == entry.key => Some(m.clone()),
                    _ => None,
                };
                let failed = match &self.detail {
                    Some(Detail::Failed(k, e)) if *k == entry.key => Some(e.clone()),
                    _ => None,
                };
                let entry_c = entry.clone();
                let preview_entry = entry.clone();
                body = body
                    .child(
                        div()
                            .id("insp-preview")
                            .cursor_pointer()
                            .child(preview_well(file_visual(entry, thumb, well - 16., false, p), well, p))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_preview(preview_entry.clone(), window, cx))),
                    )
                    .child(title_block(entry.name.clone(), util::kind_label(&entry.name), p))
                    .child(divider(p))
                    .child(
                        section(IconName::Info, "Details", p)
                            .child(row("Size", util::human_size(entry.size), p))
                            .child(row("Extension", if ext.is_empty() { "—".into() } else { ext }, p))
                            .child(row("Kind", meta.as_ref().and_then(|m| m.content_type.clone()).unwrap_or_else(|| util::guess_mime(&entry.name).to_string()), p))
                            .child(row("Storage class", entry.storage_class.clone().unwrap_or_else(|| "STANDARD".into()), p)),
                    )
                    .child(section(IconName::CalendarClock, "Dates", p).child(row("Modified", util::full_time(entry.modified), p)))
                    .child(
                        section(IconName::HardDrive, "Storage", p)
                            .child(row("Bucket", bucket.clone(), p))
                            .child(row("Key", entry.key.clone(), p))
                            .child(row("ETag", entry.etag.clone().unwrap_or_else(|| "—".into()), p))
                            .child(
                                h_flex()
                                    .gap(px(12.))
                                    .text_size(px(12.))
                                    .child(div().text_color(p.ink_faint).child("Local copy"))
                                    .child(div().flex_1())
                                    .child(if cached {
                                        h_flex().gap(px(5.)).text_color(p.success).child(icon(IconName::CircleCheck, 12., p.success)).child("Cached — no egress")
                                    } else {
                                        h_flex().text_color(p.ink_dull).child("Not cached")
                                    }),
                            ),
                    )
                    .when_some(meta, |d, m| {
                        let mut s = section(gpui_kit::assets::IconName::Tag, "Metadata", p);
                        if let Some(v) = m.cache_control.clone() {
                            s = s.child(row("Cache-Control", v, p));
                        }
                        if let Some(v) = m.server_side_encryption.clone() {
                            s = s.child(row("Encryption", v, p));
                        }
                        if let Some(v) = m.version_id.clone().filter(|v| v != "null") {
                            s = s.child(row("Version", v, p));
                        }
                        for (k, v) in &m.metadata {
                            s = s.child(row(k, v.clone(), p));
                        }
                        let empty = m.cache_control.is_none() && m.server_side_encryption.is_none() && m.metadata.is_empty();
                        if empty { d } else { d.child(s) }
                    })
                    .when_some(failed, |d, e| d.child(div().text_size(px(12.)).text_color(p.danger).child(e)));

                let (k1, k2) = (entry.key.clone(), entry.key.clone());
                let open_e = entry_c.clone();
                let ql = entry_c.clone();
                let link_key = entry.key.clone();
                let entity = cx.entity();
                actions = actions
                    .child(tool_button("insp-ql", IconName::ScanEye, "Quick Look  ␣").on_click(cx.listener(move |this, _, window, cx| this.open_preview(ql.clone(), window, cx))))
                    .child(tool_button("insp-open", IconName::SquareArrowOutUpRight, "Open with default app").on_click(cx.listener(move |this, _, window, cx| this.open_external(open_e.clone(), window, cx))))
                    .child(tool_button("insp-dl", IconName::Download, "Download…").on_click(cx.listener(|this, _, window, cx| this.download_selected(window, cx))))
                    .child(
                        Button::new("insp-link").ghost().small().icon(IconName::Link).tooltip("Copy share link").dropdown_menu(move |menu, _, _| {
                            let mut menu = menu.label("Pre-signed link valid for");
                            for (label, secs) in [("1 hour", 3600u64), ("24 hours", 86400), ("7 days", 604800)] {
                                let (e, k) = (entity.clone(), link_key.clone());
                                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| e.update(cx, |this, cx| this.copy_presigned(k.clone(), secs, window, cx))));
                            }
                            menu
                        }),
                    )
                    .child(tool_button("insp-copy", IconName::Copy, "Copy S3 URI").on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(uri) = this.s3_uri(&k1) {
                            this.copy_text(uri, "S3 URI", window, cx);
                        }
                    })))
                    .child(tool_button("insp-del", IconName::Trash, "Delete…").on_click(cx.listener(move |this, _, window, cx| {
                        let _ = &k2;
                        this.delete_selected(window, cx)
                    })));
            }
            many => {
                let size: u64 = many.iter().map(|e| e.size).sum();
                let folders = many.iter().filter(|e| e.is_dir).count();
                // A few small file reads at most; skip for huge selections.
                let cached = (many.len() <= 50).then(|| many.iter().filter(|e| self.is_cached(e)).count());
                // A little stack of the first three items.
                let stack = div()
                    .relative()
                    .size(px(well * 0.6))
                    .children(many.iter().take(3).enumerate().map(|(i, e)| {
                        let t = self.thumb_for(e);
                        div()
                            .absolute()
                            .top(px(i as f32 * 10.))
                            .left(px(i as f32 * 14.))
                            .child(file_visual(e, t, well * 0.42, false, p))
                    }));
                body = body
                    .child(preview_well(stack.into_any_element(), well, p))
                    .child(title_block(format!("{} items", many.len()), "Selection".into(), p))
                    .child(divider(p))
                    .child(
                        section(IconName::Info, "Details", p)
                            .child(row("Files", (many.len() - folders).to_string(), p))
                            .child(row("Folders", folders.to_string(), p))
                            .child(row("Size (files)", util::human_size(size), p))
                            .when_some(cached, |d, n| d.child(row("Cached locally", n.to_string(), p))),
                    );
                actions = actions
                    .child(tool_button("insp-dl", IconName::Download, "Download all…").on_click(cx.listener(|this, _, window, cx| this.download_selected(window, cx))))
                    .child(tool_button("insp-del", IconName::Trash, "Delete…").on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))));
            }
        }

        div()
            .p(px(GAP))
            .pl(px(0.))
            .h_full()
            .flex_shrink_0()
            .child(
                v_flex()
                    .w(px(INSPECTOR_W))
                    .h_full()
                    .rounded(px(12.))
                    .bg(p.panel)
                    .border_1()
                    .border_color(p.panel_border)
                    .overflow_hidden()
                    .child(v_flex().id("inspector-scroll").flex_1().min_h_0().overflow_y_scroll().p(px(16.)).child(body))
                    .child(h_flex().h(px(44.)).flex_shrink_0().border_t_1().border_color(p.line).justify_center().child(actions)),
            )
            .into_any_element()
    }
}

fn preview_well(content: AnyElement, size: f32, p: &Palette) -> Div {
    div()
        .w(px(size))
        .h(px(size))
        .rounded(px(12.))
        .bg(p.well)
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .child(content)
}

fn title_block(name: String, kind: String, p: &Palette) -> Div {
    v_flex()
        .items_center()
        .gap(px(2.))
        .child(div().max_w_full().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(name))
        .child(div().text_size(px(11.5)).text_color(p.ink_faint).child(kind))
}
