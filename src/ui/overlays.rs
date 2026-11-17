//! Quick Preview, the floating transfers panel and the cache settings dialog.

use std::path::Path;

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, FontWeight, ObjectFit, SharedString, Window, div, img, px};

use super::{caption, file_visual, icon, tool_button};
use crate::app::{PreviewState, S3App};
use crate::cache;
use crate::theme::{self, Palette};
use crate::transfers::{Direction, Status};
use crate::util;

impl S3App {
    // ---- Quick Preview --------------------------------------------------------

    pub(super) fn render_preview(&mut self, p: &Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let preview = self.preview.as_ref().unwrap();
        let files: Vec<_> = self.visible(cx).into_iter().filter(|e| !e.is_dir).collect();
        let index = files.iter().position(|e| e.key == preview.key);
        let entry = index.map(|i| files[i].clone());
        let Some(entry) = entry else {
            return div().into_any_element();
        };
        let viewport = window.viewport_size();
        let (vw, vh): (f32, f32) = (viewport.width.into(), viewport.height.into());
        let (max_w, max_h) = (vw - 160., vh - 150.);
        let thumb = self.thumb_for(&entry);

        let stage: AnyElement = match &preview.state {
            PreviewState::Ready(path) => img(path.clone())
                .object_fit(ObjectFit::Contain)
                .w(px(max_w))
                .h(px(max_h))
                .rounded(px(6.))
                .into_any_element(),
            PreviewState::Loading => v_flex()
                .items_center()
                .gap(px(14.))
                .child(match thumb.clone() {
                    // Show the thumbnail while the full image arrives.
                    Some(t) => img(t).object_fit(ObjectFit::Contain).w(px(max_w.min(640.))).h(px(max_h.min(480.))).rounded(px(6.)).into_any_element(),
                    None => file_visual(&entry, None, 180., false, p),
                })
                .child(h_flex().gap(px(8.)).text_color(p.ink_dull).child(icon(IconName::LoaderCircle, 14., p.ink_dull)).child("Loading full image…"))
                .into_any_element(),
            other => {
                let note = match other {
                    PreviewState::TooLarge => format!("{} — too large to preview here", util::human_size(entry.size)),
                    PreviewState::Failed(e) => format!("Couldn't load: {e}"),
                    _ => "Open it in its default app to see the full file".to_string(),
                };
                let open_e = entry.clone();
                v_flex()
                    .items_center()
                    .gap(px(16.))
                    .child(match thumb.clone() {
                        Some(t) => img(t).object_fit(ObjectFit::Contain).w(px(max_w.min(560.))).h(px(max_h.min(420.))).rounded(px(6.)).into_any_element(),
                        None => file_visual(&entry, None, 200., false, p),
                    })
                    .child(div().text_color(p.ink_dull).child(note))
                    .child(
                        Button::new("preview-open-ext")
                            .primary()
                            .icon(IconName::SquareArrowOutUpRight)
                            .label("Open with default app")
                            .on_click(cx.listener(move |this, _, window, cx| this.open_external(open_e.clone(), window, cx))),
                    )
                    .into_any_element()
            }
        };

        let counter = index.map(|i| format!("{} of {}", i + 1, files.len())).unwrap_or_default();
        let cached = self.is_cached(&entry);
        let open_e = entry.clone();
        let nav_button = |id: &'static str, name: IconName, delta: i64, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(44.))
                .rounded_full()
                .bg(p.panel.opacity(0.85))
                .border_1()
                .border_color(p.panel_border)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(p.hover))
                .child(icon(name, 20., p.ink))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.step_preview(delta, window, cx)
                }))
        };

        div()
            .id("preview-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(p.backdrop)
            .text_color(if p.dark { p.ink } else { gpui_kit::white() })
            .on_click(cx.listener(|this, _, _, cx| this.close_preview(cx)))
            .child(
                v_flex()
                    .size_full()
                    .child(
                        h_flex()
                            .id("preview-bar")
                            .h(px(56.))
                            .px(px(20.))
                            .pl(px(90.))
                            .gap(px(10.))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(entry.name.clone()))
                                    .child(div().text_size(px(11.5)).text_color(p.ink_faint).child(format!(
                                        "{} · {}{}",
                                        counter,
                                        util::human_size(entry.size),
                                        if cached { " · cached locally" } else { "" }
                                    ))),
                            )
                            .child(
                                Button::new("preview-dl")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Download)
                                    .label("Download")
                                    .on_click(cx.listener(|this, _, window, cx| this.download_selected(window, cx))),
                            )
                            .child(
                                Button::new("preview-open")
                                    .ghost()
                                    .small()
                                    .icon(IconName::SquareArrowOutUpRight)
                                    .label("Open")
                                    .on_click(cx.listener(move |this, _, window, cx| this.open_external(open_e.clone(), window, cx))),
                            )
                            .child(tool_button("preview-close", IconName::X, "Close  esc").on_click(cx.listener(|this, _, _, cx| this.close_preview(cx)))),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .px(px(20.))
                            .gap(px(16.))
                            .child(nav_button("preview-prev", IconName::ChevronLeft, -1, cx))
                            .child(
                                div()
                                    .id("preview-stage")
                                    .flex_1()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(|_, _, cx| cx.stop_propagation())
                                    .child(stage),
                            )
                            .child(nav_button("preview-next", IconName::ChevronRight, 1, cx)),
                    )
                    .child(
                        div()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.5))
                            .text_color(p.ink_faint)
                            .child("← → to browse  ·  space or esc to close"),
                    ),
            )
            .into_any_element()
    }

    // ---- transfers -----------------------------------------------------------

    pub(super) fn render_transfers(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let active = self.transfers.iter().filter(|t| t.is_active()).count();
        let rows = self.transfers.iter().take(200).map(|t| {
            let status = t.status();
            let fraction = t.fraction();
            let done = t.done.load(std::sync::atomic::Ordering::Relaxed);
            let total = t.total.load(std::sync::atomic::Ordering::Relaxed);
            let (text, color) = match &status {
                Status::Queued => ("Queued".to_string(), p.ink_faint),
                Status::Running => (
                    format!("{} of {} · {}/s", util::human_size(done), util::human_size(total), util::human_size(t.speed() as u64)),
                    p.ink_faint,
                ),
                Status::Done => (format!("Done · {}", util::human_size(total)), p.success),
                Status::Failed(e) => (format!("Failed: {e}"), p.danger),
                Status::Cancelled => ("Cancelled".to_string(), p.ink_faint),
            };
            let tt = t.clone();
            let local = t.local.clone();
            let reveal = status == Status::Done && t.direction == Direction::Download && Path::new(&local).is_absolute();
            h_flex()
                .id(SharedString::from(format!("transfer-{}", t.id)))
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .hover(|s| s.bg(p.hover))
                .child(
                    div()
                        .size(px(30.))
                        .rounded(px(8.))
                        .bg(p.well)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(
                            if t.direction == Direction::Upload { IconName::Upload } else { IconName::Download },
                            14.,
                            if t.direction == Direction::Upload { p.accent } else { p.success },
                        )),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(3.))
                        .child(div().truncate().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(t.name.clone()))
                        .when(matches!(status, Status::Running | Status::Queued), |d| {
                            d.child(
                                Progress::new(SharedString::from(format!("p-{}", t.id)))
                                    .value(fraction * 100.)
                                    .loading(total == 0 && status == Status::Running),
                            )
                        })
                        .child(div().truncate().text_size(px(11.)).text_color(color).child(text)),
                )
                .when(t.is_active(), |d| {
                    d.child(tool_button(SharedString::from(format!("cancel-{}", t.id)), IconName::X, "Cancel").on_click(move |_, _, _| tt.cancel()))
                })
                .when(reveal, |d| {
                    d.child(
                        tool_button(SharedString::from(format!("reveal-{}", t.id)), IconName::FolderOpen, "Show in Finder")
                            .on_click(move |_, _, cx| cx.reveal_path(Path::new(&local))),
                    )
                })
        });

        div()
            .absolute()
            .right(px(if self.settings.inspector && self.session.as_ref().is_some_and(|s| s.bucket.is_some()) { super::INSPECTOR_W + 24. } else { 16. }))
            .bottom(px(40.))
            .w(px(380.))
            .child(
                v_flex()
                    .id("transfers-panel")
                    .occlude()
                    .max_h(px(380.))
                    .rounded(px(12.))
                    .bg(p.panel)
                    .border_1()
                    .border_color(p.panel_border)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(42.))
                            .px(px(12.))
                            .gap(px(8.))
                            .border_b_1()
                            .border_color(p.line)
                            .child(icon(IconName::ArrowUpDown, 14., p.accent))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Transfers"))
                            .when(active > 0, |d| d.child(div().text_size(px(11.5)).text_color(p.accent).child(format!("{active} active"))))
                            .child(div().flex_1())
                            .when(active > 0, |d| {
                                d.child(Button::new("cancel-all").ghost().xsmall().label("Cancel all").on_click(cx.listener(|this, _, _, cx| {
                                    for t in &this.transfers {
                                        t.cancel();
                                    }
                                    cx.notify();
                                })))
                            })
                            .child(Button::new("clear-transfers").ghost().xsmall().label("Clear").on_click(cx.listener(|this, _, _, cx| this.clear_finished_transfers(cx))))
                            .child(tool_button("hide-transfers", IconName::X, "Hide").on_click(cx.listener(|this, _, _, cx| {
                                this.show_transfers = false;
                                cx.notify();
                            }))),
                    )
                    .child(
                        v_flex()
                            .id("transfer-list")
                            .p(px(4.))
                            .overflow_y_scroll()
                            .when(self.transfers.is_empty(), |d| {
                                d.child(div().p(px(16.)).text_size(px(12.5)).text_color(p.ink_faint).child("Uploads and downloads show up here."))
                            })
                            .children(rows),
                    ),
            )
            .into_any_element()
    }

    // ---- cache & previews settings -------------------------------------------

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = cx.entity();
        // Walking the cache folder isn't free: measure once per opening.
        let usage = std::rc::Rc::new(std::cell::Cell::new(cache::global().usage()));
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = theme::palette(cx);
            let s = app.read(cx).settings.clone();
            let stats = cache::global().stats();
            let u = usage.get();
            let chips = |id: &'static str, options: &[(u64, &'static str)], current: u64, apply: fn(&mut S3App, u64)| {
                let mut row = h_flex().gap(px(6.));
                for (value, label) in options.iter().copied() {
                    let app = app.clone();
                    let selected = value == current;
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("{id}-{value}")))
                            .px(px(10.))
                            .py(px(5.))
                            .rounded(px(7.))
                            .border_1()
                            .text_size(px(12.))
                            .cursor_pointer()
                            .when(selected, |d| d.bg(p.accent).border_color(p.accent).text_color(p.on_accent))
                            .when(!selected, |d| d.border_color(p.line).hover(|s| s.bg(p.hover)))
                            .child(label)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    apply(this, value);
                                    this.save_settings(cx);
                                    this.retry_thumbnails(cx);
                                })
                            }),
                    );
                }
                row
            };
            let app_toggle = app.clone();
            let usage_clear = usage.clone();
            let app_clear = app.clone();
            dialog
                .w(px(520.))
                .title(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Cache & previews"))
                .child(
                    v_flex()
                        .gap(px(18.))
                        .child(
                            h_flex()
                                .gap(px(14.))
                                .p(px(14.))
                                .rounded(px(10.))
                                .bg(p.accent_soft)
                                .child(icon(IconName::PiggyBank, 26., p.accent))
                                .child(
                                    v_flex()
                                        .gap(px(2.))
                                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(format!("{} served from cache", util::human_size(stats.saved))))
                                        .child(div().text_size(px(12.)).text_color(p.ink_dull).child(format!(
                                            "Instead of downloading it again. {} fetched from storage in total.",
                                            util::human_size(stats.fetched)
                                        ))),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(px(12.))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(div().font_weight(FontWeight::MEDIUM).child("Image thumbnails"))
                                        .child(div().text_size(px(12.)).text_color(p.ink_faint).child("Fetch images once to show previews in the grid. Each thumbnail is kept, so it's never fetched again.")),
                                )
                                .child(Switch::new("previews").checked(s.previews).on_click(move |checked: &bool, _, cx| {
                                    let checked = *checked;
                                    app_toggle.update(cx, |this, cx| {
                                        this.settings.previews = checked;
                                        this.save_settings(cx);
                                        this.retry_thumbnails(cx);
                                    })
                                })),
                        )
                        .child(
                            v_flex()
                                .gap(px(8.))
                                .child(caption("Largest image to thumbnail", &p))
                                .child(chips("pmax", &[(5, "5 MB"), (15, "15 MB"), (50, "50 MB"), (200, "200 MB")], s.preview_max_mb, |t, v| t.settings.preview_max_mb = v))
                                .child(div().text_size(px(11.5)).text_color(p.ink_faint).child("Bigger images show a file icon until you open them — keeps egress low on folders of RAW photos.")),
                        )
                        .child(
                            v_flex()
                                .gap(px(8.))
                                .child(caption("Cache size limit", &p))
                                .child(chips("cmax", &[(1, "1 GB"), (5, "5 GB"), (20, "20 GB"), (50, "50 GB")], s.cache_gb, |t, v| t.settings.cache_gb = v))
                                .child(div().text_size(px(11.5)).text_color(p.ink_faint).child(format!(
                                    "Using {} for {} files and {} for {} thumbnails. Least recently used files go first.",
                                    util::human_size(u.objects_bytes),
                                    u.objects,
                                    util::human_size(u.thumbs_bytes),
                                    u.thumbs
                                ))),
                        ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .gap(px(8.))
                        .child(Button::new("cache-reveal").ghost().icon(IconName::FolderOpen).label("Show in Finder").on_click(|_, _, cx| {
                            cx.reveal_path(cache::global().root());
                        }))
                        .child(Button::new("cache-clear").ghost().icon(IconName::Trash).label("Clear cache").on_click(move |_, window, cx| {
                            match cache::global().clear() {
                                Ok(()) => {
                                    usage_clear.set(cache::global().usage());
                                    app_clear.update(cx, |this, cx| {
                                        this.thumbs.clear();
                                        cx.notify();
                                    });
                                    S3App::notify_ok(window, cx, "Cache cleared");
                                }
                                Err(e) => S3App::notify_err(window, cx, format!("Couldn't clear the cache: {e}")),
                            }
                        }))
                        .child(div().flex_1())
                        .child(Button::new("settings-done").primary().label("Done").on_click(|_, window, cx| window.close_dialog(cx))),
                )
        });
    }
}
