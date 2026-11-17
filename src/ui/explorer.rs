//! Top bar, the explorer views (grid / list / media), path bar, welcome and
//! bucket overview.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, ExternalPaths, FontWeight, MouseButton, SharedString, Window, div, img, px, uniform_list};

use super::{GAP, INSPECTOR_W, SIDEBAR_W, caption, drag_area, empty_state, file_visual, icon, segmented, tool_button};
use crate::app::{S3App, SortKey, Thumb, folder_key, thumb_key};
use crate::cache;
use crate::connection_form::provider_badge;
use crate::providers::PROVIDERS;
use crate::s3::Entry;
use crate::settings::ViewMode;
use crate::theme::Palette;
use crate::util;

/// (thumbnail box, tile width) for the grid zoom steps.
fn grid_metrics(zoom: u8) -> (f32, f32) {
    match zoom {
        0 => (76., 112.),
        2 => (150., 186.),
        _ => (108., 144.),
    }
}

fn media_tile(zoom: u8) -> f32 {
    match zoom {
        0 => 96.,
        2 => 200.,
        _ => 140.,
    }
}

impl S3App {
    // ---- top bar ---------------------------------------------------------------

    pub(super) fn render_topbar(&mut self, p: &Palette, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.as_ref();
        let in_bucket = session.is_some_and(|s| s.bucket.is_some());
        let can_back = session.is_some_and(|s| !s.back.is_empty());
        let can_fwd = session.is_some_and(|s| !s.forward.is_empty());
        let loading = session.is_some_and(|s| s.loading);

        let (title, is_bucket_root) = match session {
            Some(s) => match &s.bucket {
                Some(b) if s.prefix.is_empty() => (b.clone(), true),
                Some(_) => (s.prefix.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string(), false),
                None => ("Buckets".to_string(), true),
            },
            None => ("S3 Browser".to_string(), true),
        };
        let location = h_flex()
            .h(px(30.))
            .px(px(10.))
            .gap(px(8.))
            .rounded(px(8.))
            .bg(p.well)
            .border_1()
            .border_color(p.line)
            .max_w(px(260.))
            .child(if session.is_none() {
                icon(IconName::Cloud, 15., p.accent).into_any_element()
            } else if is_bucket_root && session.is_some_and(|s| s.bucket.is_some()) {
                img("art/bucket.svg").size(px(16.)).into_any_element()
            } else if is_bucket_root {
                icon(IconName::Boxes, 15., p.accent).into_any_element()
            } else {
                img("art/folder.svg").w(px(17.)).h(px(14.)).into_any_element()
            })
            .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(title))
            .when(loading, |d| d.child(icon(IconName::LoaderCircle, 13., p.ink_faint)));

        let view = self.settings.view;
        let entity = cx.entity();
        let views = {
            let (e1, e2, e3) = (entity.clone(), entity.clone(), entity.clone());
            segmented(
                vec![
                    (
                        IconName::LayoutGrid,
                        "Grid",
                        view == ViewMode::Grid,
                        Box::new(move |_: &mut Window, cx: &mut App| e1.update(cx, |t, cx| t.set_view(ViewMode::Grid, cx))),
                    ),
                    (
                        IconName::List,
                        "List",
                        view == ViewMode::List,
                        Box::new(move |_: &mut Window, cx: &mut App| e2.update(cx, |t, cx| t.set_view(ViewMode::List, cx))),
                    ),
                    (
                        IconName::Images,
                        "Media",
                        view == ViewMode::Media,
                        Box::new(move |_: &mut Window, cx: &mut App| e3.update(cx, |t, cx| t.set_view(ViewMode::Media, cx))),
                    ),
                ],
                p,
            )
        };

        let sort = self.sort;
        let asc = self.sort_asc;
        let sort_menu = Button::new("sort").ghost().small().icon(if asc { IconName::ArrowUpNarrowWide } else { IconName::ArrowDownWideNarrow }).tooltip("Sort").dropdown_menu({
            let e = entity.clone();
            move |menu: PopupMenu, _, _| {
                let mut menu = menu.label("Sort by");
                for (label, key) in [("Name", SortKey::Name), ("Size", SortKey::Size), ("Kind", SortKey::Kind), ("Date modified", SortKey::Modified)] {
                    let e = e.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(sort == key).on_click(move |_, _, cx| {
                        e.update(cx, |t, cx| {
                            if t.sort != key {
                                t.set_sort(key, cx)
                            }
                        })
                    }));
                }
                let (ea, ed) = (e.clone(), e.clone());
                menu.separator()
                    .item(PopupMenuItem::new("Ascending").checked(asc).on_click(move |_, _, cx| {
                        ea.update(cx, |t, cx| {
                            t.sort_asc = true;
                            cx.notify()
                        })
                    }))
                    .item(PopupMenuItem::new("Descending").checked(!asc).on_click(move |_, _, cx| {
                        ed.update(cx, |t, cx| {
                            t.sort_asc = false;
                            cx.notify()
                        })
                    }))
            }
        });

        let upload = Button::new("upload")
            .primary()
            .small()
            .icon(IconName::Upload)
            .label("Upload")
            .dropdown_caret(true)
            .dropdown_menu({
                let e = entity.clone();
                move |menu, _, _| {
                    let (a, b) = (e.clone(), e.clone());
                    menu.item(PopupMenuItem::new("Files…").icon(IconName::FileUp).on_click(move |_, window, cx| {
                        a.update(cx, |this, cx| this.upload_dialog(false, window, cx))
                    }))
                    .item(PopupMenuItem::new("Folder…").icon(IconName::FolderInput).on_click(move |_, window, cx| {
                        b.update(cx, |this, cx| this.upload_dialog(true, window, cx))
                    }))
                }
            });

        h_flex()
            .h(px(super::TOPBAR_H))
            .flex_shrink_0()
            .px(px(10.))
            .gap(px(6.))
            // Traffic lights sit here when the sidebar is hidden.
            .when(!self.settings.sidebar, |d| {
                d.pl(px(80.)).child(tool_button("show-sidebar", IconName::PanelLeftOpen, "Show sidebar").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.settings.sidebar = true;
                        this.save_settings(cx);
                    },
                )))
            })
            .when(session.is_some(), |d| {
                d.child(
                    h_flex()
                        .child(
                            tool_button("back", IconName::ChevronLeft, "Back  ⌘[")
                                .disabled(!can_back)
                                .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))),
                        )
                        .child(
                            tool_button("forward", IconName::ChevronRight, "Forward  ⌘]")
                                .disabled(!can_fwd)
                                .on_click(cx.listener(|this, _, window, cx| this.go_forward(window, cx))),
                        ),
                )
            })
            .child(location)
            .child(drag_area("top-drag-1").flex_1().h_full().min_w(px(12.)))
            .when(in_bucket, |d| {
                d.child(
                    div().w(px(240.)).child(
                        Input::new(&self.filter)
                            .small()
                            .cleanable(true)
                            .prefix(icon(IconName::Search, 13., p.ink_faint)),
                    ),
                )
            })
            .child(drag_area("top-drag-2").flex_1().h_full().min_w(px(12.)))
            .when(in_bucket, |d| {
                d.child(upload)
                    .child(
                        tool_button("new-folder", IconName::FolderPlus, "New folder")
                            .on_click(cx.listener(|this, _, window, cx| this.new_folder(window, cx))),
                    )
                    .child(
                        tool_button("refresh", IconName::RefreshCw, "Refresh  ⌘R")
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
                    .child(div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(p.line))
                    .child(views)
                    .when(view != ViewMode::List, |d| {
                        d.child(
                            tool_button("zoom-out", IconName::ZoomOut, "Smaller")
                                .disabled(self.settings.zoom == 0)
                                .on_click(cx.listener(|this, _, _, cx| this.zoom(-1, cx))),
                        )
                        .child(
                            tool_button("zoom-in", IconName::ZoomIn, "Larger")
                                .disabled(self.settings.zoom == 2)
                                .on_click(cx.listener(|this, _, _, cx| this.zoom(1, cx))),
                        )
                    })
                    .child(sort_menu)
                    .child(
                        tool_button("inspector", IconName::PanelRight, "Inspector")
                            .toggled(self.settings.inspector)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.inspector = !this.settings.inspector;
                                this.save_settings(cx);
                            })),
                    )
            })
            .into_any_element()
    }

    // ---- content -----------------------------------------------------------------

    pub(super) fn render_content(&mut self, p: &Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.session {
            None => self.render_welcome(p, cx),
            Some(s) if s.bucket.is_none() => self.render_overview(p, cx),
            Some(_) => self.render_explorer(p, window, cx),
        }
    }

    fn render_explorer(&mut self, p: &Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.as_ref().unwrap();
        let entries = Rc::new(self.visible(cx));
        let count = entries.len();
        let filtering = !self.filter.read(cx).value().trim().is_empty();

        let body: AnyElement = if let Some(err) = session.error.clone().filter(|_| count == 0) {
            empty_state(IconName::CloudOff, "Couldn't list this folder", &err, p)
        } else if count == 0 && session.loading {
            empty_state(IconName::LoaderCircle, "Loading…", "", p)
        } else if count == 0 && filtering {
            empty_state(IconName::Search, "No matches", "Nothing in this folder matches your search.", p)
        } else if count == 0 && self.settings.view == ViewMode::Media {
            empty_state(IconName::Images, "No media here", "Photos and videos in this folder show up in Media view.", p)
        } else if count == 0 {
            empty_state(IconName::FolderOpen, "This folder is empty", "Drop files here or use Upload.", p)
        } else {
            match self.settings.view {
                ViewMode::List => self.render_list(entries, p, cx),
                ViewMode::Grid => self.render_grid(entries, false, p, window, cx),
                ViewMode::Media => self.render_grid(entries, true, p, window, cx),
            }
        };

        let (accent, soft) = (p.accent, p.accent_soft);
        div()
            .id("drop-zone")
            .size_full()
            .flex()
            .flex_col()
            .border_2()
            .rounded(px(10.))
            .border_color(gpui_kit::transparent_black())
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(accent).bg(soft))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.upload_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(body)
            .into_any_element()
    }

    /// Ask for thumbnails of what's on screen, after this frame is drawn.
    fn thumb_requester(&self, cx: &mut Context<Self>) -> impl Fn(Vec<Entry>, &mut Window, &mut App) + 'static {
        let view = cx.entity();
        let known: Rc<std::collections::HashSet<String>> = Rc::new(self.thumbs.keys().cloned().collect());
        let enabled = self.settings.previews;
        let scheduled = Rc::new(Cell::new(false));
        let wanted: Rc<RefCell<Vec<Entry>>> = Rc::new(RefCell::new(Vec::new()));
        move |visible: Vec<Entry>, window: &mut Window, cx: &mut App| {
            if !enabled {
                return;
            }
            let missing: Vec<Entry> = visible
                .into_iter()
                .filter(|e| !e.is_dir && cache::is_previewable(&e.name) && !known.contains(&thumb_key(e)))
                .collect();
            if missing.is_empty() {
                return;
            }
            wanted.borrow_mut().extend(missing);
            if !scheduled.replace(true) {
                let (view, wanted, scheduled) = (view.clone(), wanted.clone(), scheduled.clone());
                window.defer(cx, move |_, cx| {
                    scheduled.set(false);
                    let batch = std::mem::take(&mut *wanted.borrow_mut());
                    view.update(cx, |this, cx| this.request_thumbs(batch, cx));
                });
            }
        }
    }

    fn render_grid(&mut self, entries: Rc<Vec<Entry>>, media: bool, p: &Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Width available to the grid: window minus sidebar and inspector.
        let mut avail: f32 = window.viewport_size().width.into();
        if self.settings.sidebar {
            avail -= SIDEBAR_W + GAP;
        }
        if self.settings.inspector {
            avail -= INSPECTOR_W + GAP * 2.;
        }
        avail -= 36.; // padding + scrollbar
        let (thumb, tile_min, row_h) = if media {
            let t = media_tile(self.settings.zoom);
            (t, t + 4., t + 4.)
        } else {
            let (t, w) = grid_metrics(self.settings.zoom);
            (t, w, t + 56.)
        };
        let cols = ((avail / tile_min).floor() as usize).max(1);
        self.grid_cols = cols;
        let tile_w = if media { (avail / cols as f32).floor() } else { (avail / cols as f32).floor().max(tile_min) };
        let thumb = if media { tile_w - 4. } else { thumb };
        let rows = entries.len().div_ceil(cols);

        let view = cx.entity();
        let selected = Rc::new(self.selected.clone());
        let thumbs: Rc<std::collections::HashMap<String, Thumb>> = Rc::new(self.thumbs.clone());
        let request = self.thumb_requester(cx);
        let p = *p;
        let sizes = Rc::new(self.folder_sizes.clone());
        let bucket = self.session.as_ref().and_then(|s| s.bucket.clone()).unwrap_or_default();

        uniform_list("grid", rows, move |range, window, cx| {
            let mut on_screen = Vec::new();
            let rows = range
                .map(|row| {
                    let start = row * cols;
                    let end = (start + cols).min(entries.len());
                    let tiles = (start..end).map(|ix| {
                        let entry = entries[ix].clone();
                        on_screen.push(entry.clone());
                        let is_sel = selected.contains(&entry.key);
                        let thumb_path = match thumbs.get(&thumb_key(&entry)) {
                            Some(Thumb::Ready(path)) => Some(path.clone()),
                            _ => None,
                        };
                        if media {
                            media_tile_el(ix, &entry, thumb_path, thumb, tile_w, is_sel, &view, &p)
                        } else {
                            let size_label = if entry.is_dir {
                                util::folder_size_label(sizes.get(&folder_key(&bucket, &entry.key)).copied())
                            } else {
                                util::human_size(entry.size)
                            };
                            grid_tile_el(ix, &entry, thumb_path, thumb, tile_w, is_sel, size_label, &view, &p)
                        }
                    });
                    h_flex().w_full().h(px(row_h)).px(px(12.)).items_start().children(tiles)
                })
                .collect::<Vec<_>>();
            request(on_screen, window, cx);
            rows
        })
        .track_scroll(&self.scroll)
        .size_full()
        .pt(px(4.))
        .into_any_element()
    }

    fn render_list(&mut self, entries: Rc<Vec<Entry>>, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let header_cell = |this: &Self, label: &'static str, key: SortKey, width: Option<f32>, cx: &mut Context<Self>| {
            let active = this.sort == key;
            let arrow = if this.sort_asc { IconName::ChevronUp } else { IconName::ChevronDown };
            h_flex()
                .id(SharedString::from(format!("sort-{label}")))
                .gap(px(4.))
                .map(|d| match width {
                    Some(w) => d.w(px(w)).flex_shrink_0(),
                    None => d.flex_1().min_w_0(),
                })
                .cursor_pointer()
                .when(active, |d| d.text_color(p.ink))
                .child(label)
                .when(active, |d| d.child(icon(arrow, 11., p.ink)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };
        let header = h_flex()
            .h(px(30.))
            .flex_shrink_0()
            .mx(px(12.))
            .px(px(10.))
            .gap(px(12.))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.ink_faint)
            .border_b_1()
            .border_color(p.line)
            .child(header_cell(self, "Name", SortKey::Name, None, cx))
            .child(header_cell(self, "Size", SortKey::Size, Some(84.), cx))
            .child(header_cell(self, "Kind", SortKey::Kind, Some(100.), cx))
            .child(header_cell(self, "Date modified", SortKey::Modified, Some(130.), cx));

        let view = cx.entity();
        let selected = Rc::new(self.selected.clone());
        let thumbs: Rc<std::collections::HashMap<String, Thumb>> = Rc::new(self.thumbs.clone());
        let request = self.thumb_requester(cx);
        let p = *p;
        let sizes = Rc::new(self.folder_sizes.clone());
        let bucket = self.session.as_ref().and_then(|s| s.bucket.clone()).unwrap_or_default();
        let list = uniform_list("list", entries.len(), move |range, window, cx| {
            let mut on_screen = Vec::new();
            let rows = range
                .map(|ix| {
                    let entry = entries[ix].clone();
                    on_screen.push(entry.clone());
                    let is_sel = selected.contains(&entry.key);
                    let thumb_path = match thumbs.get(&thumb_key(&entry)) {
                        Some(Thumb::Ready(path)) => Some(path.clone()),
                        _ => None,
                    };
                    let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
                    let (e1, key) = (entry.clone(), entry.key.clone());
                    h_flex()
                        .id(ix)
                        .w_full()
                        .h(px(32.))
                        .px(px(10.))
                        .gap(px(12.))
                        .rounded(px(7.))
                        .when(is_sel, |d| d.bg(p.accent).text_color(p.on_accent))
                        .when(!is_sel && ix % 2 == 1, |d| d.bg(p.hover.opacity(0.45)))
                        .when(!is_sel, |d| d.hover(|s| s.bg(p.hover)))
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(9.))
                                .child(file_visual(&entry, thumb_path, 22., false, &p))
                                .child(div().min_w_0().truncate().child(entry.name.clone())),
                        )
                        .child(
                            div()
                                .w(px(84.))
                                .flex_shrink_0()
                                .when(!is_sel, |d| d.text_color(p.ink_dull))
                                .child(if entry.is_dir {
                                    Some(util::folder_size_label(sizes.get(&folder_key(&bucket, &entry.key)).copied()))
                                        .filter(|l| !l.is_empty())
                                        .unwrap_or_else(|| "—".to_string())
                                } else {
                                    util::human_size(entry.size)
                                }),
                        )
                        .child(
                            div()
                                .w(px(100.))
                                .flex_shrink_0()
                                .truncate()
                                .when(!is_sel, |d| d.text_color(p.ink_dull))
                                .child(if entry.is_dir { "Folder".to_string() } else { util::kind_label(&entry.name) }),
                        )
                        .child(
                            div()
                                .w(px(130.))
                                .flex_shrink_0()
                                .when(!is_sel, |d| d.text_color(p.ink_dull))
                                .child(util::human_time(entry.modified)),
                        )
                        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                            v2.update(cx, |this, cx| this.ensure_selected(ix, &key, window, cx));
                        })
                        .on_click(move |event, window, cx| {
                            let e = e1.clone();
                            v1.update(cx, |this, cx| this.click_entry(ix, e, event, window, cx));
                        })
                        .context_menu(move |menu, _, cx| entry_menu(menu, v3.clone(), cx))
                })
                .collect::<Vec<_>>();
            request(on_screen, window, cx);
            rows
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .px(px(12.))
        .pt(px(4.));

        v_flex().size_full().child(header).child(list).into_any_element()
    }

    // ---- path bar -----------------------------------------------------------------

    pub(super) fn render_pathbar(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.as_ref().unwrap();
        let bucket = session.bucket.clone().unwrap_or_default();
        let mut crumbs: Vec<(String, String)> = vec![(bucket.clone(), String::new())];
        let mut acc = String::new();
        for part in session.prefix.split('/').filter(|s| !s.is_empty()) {
            acc.push_str(part);
            acc.push('/');
            crumbs.push((part.to_string(), acc.clone()));
        }
        let last = crumbs.len() - 1;
        let crumb_els = crumbs.into_iter().enumerate().flat_map(|(i, (label, prefix))| {
            let sep = (i > 0).then(|| icon(IconName::ChevronRight, 11., p.ink_faint).into_any_element());
            let is_last = i == last;
            let el = h_flex()
                .id(SharedString::from(format!("crumb-{i}")))
                .gap(px(5.))
                .px(px(5.))
                .py(px(2.))
                .rounded(px(5.))
                .when(is_last, |d| d.text_color(p.ink))
                .when(!is_last, |d| d.cursor_pointer().hover(|s| s.bg(p.hover).text_color(p.ink)))
                .child(if i == 0 {
                    img("art/bucket.svg").size(px(12.)).into_any_element()
                } else {
                    img("art/folder.svg").w(px(13.)).h(px(10.)).into_any_element()
                })
                .child(label)
                .when(!is_last, |d| d.on_click(cx.listener(move |this, _, window, cx| this.navigate(prefix.clone(), window, cx))))
                .into_any_element();
            sep.into_iter().chain(std::iter::once(el))
        });

        let total = session.entries.len();
        let size: u64 = session.entries.iter().map(|e| e.size).sum();
        let selected = self.selected.len();
        let active = self.transfers.iter().filter(|t| t.is_active()).count();
        let has_more = session.next_token.is_some();
        let loading_more = session.loading_more;

        h_flex()
            .h(px(30.))
            .flex_shrink_0()
            .mx(px(GAP))
            .px(px(8.))
            .gap(px(2.))
            .text_size(px(11.5))
            .text_color(p.ink_dull)
            .border_t_1()
            .border_color(p.line)
            .child(h_flex().id("crumbs").flex_1().min_w_0().overflow_x_scroll().children(crumb_els))
            .when(has_more, |d| {
                d.child(
                    Button::new("load-more")
                        .link()
                        .xsmall()
                        .label("Load more")
                        .loading(loading_more)
                        .on_click(cx.listener(|this, _, window, cx| this.load_page(true, window, cx))),
                )
            })
            .child(div().px(px(8.)).child(if selected > 0 {
                format!("{selected} of {total} selected")
            } else {
                format!("{total} items · {}{}", util::human_size(size), if has_more { "+" } else { "" })
            }))
            .when(active > 0, |d| {
                d.child(
                    h_flex()
                        .id("pathbar-transfers")
                        .gap(px(5.))
                        .px(px(8.))
                        .py(px(2.))
                        .rounded_full()
                        .bg(p.accent_soft)
                        .text_color(p.accent)
                        .cursor_pointer()
                        .child(icon(IconName::LoaderCircle, 11., p.accent))
                        .child(format!("{active} transferring"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_transfers = true;
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    // ---- welcome & overview ------------------------------------------------------

    fn render_welcome(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let cards = PROVIDERS.iter().map(|info| {
            let kind = info.kind;
            v_flex()
                .id(SharedString::from(format!("welcome-{}", info.short)))
                .w(px(180.))
                .h(px(124.))
                .gap(px(8.))
                .p(px(14.))
                .rounded(px(12.))
                .bg(p.panel)
                .border_1()
                .border_color(p.panel_border)
                .cursor_pointer()
                .hover(|s| s.border_color(p.accent).bg(p.hover))
                .child(provider_badge(kind, 32.))
                .child(div().w_full().truncate().font_weight(FontWeight::SEMIBOLD).child(info.name))
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(p.ink_faint)
                        .child(if info.endpoint.is_empty() { "Regional endpoint".to_string() } else { info.endpoint.replace("https://", "") }),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.open_connection_dialog(None, Some(kind), window, cx)))
        });
        v_flex()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .items_center()
            .pt(px(36.))
            .pb(px(48.))
            .px(px(32.))
            .gap(px(28.))
            .child(
                v_flex()
                    .items_center()
                    .gap(px(12.))
                    .child(img("art/bucket.svg").size(px(72.)))
                    .child(div().text_size(px(26.)).font_weight(FontWeight::BOLD).child("Your buckets, like a local drive"))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .text_color(p.ink_dull)
                            .max_w(px(560.))
                            .text_center()
                            .child("Browse Amazon S3 and every S3-compatible store with thumbnails and Quick Look. Everything you open is cached locally, so you never pay egress for the same file twice."),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(10.))
                    .items_center()
                    .child(caption("Connect a provider", p))
                    .child(h_flex().flex_wrap().justify_center().gap(px(10.)).max_w(px(1000.)).children(cards)),
            )
            .into_any_element()
    }

    fn render_overview(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.as_ref().unwrap();
        let tiles = session.buckets.iter().map(|b| {
            let name = b.name.clone();
            let created = b.created.map(|t| util::human_time(Some(t))).unwrap_or_else(|| "Bucket".into());
            v_flex()
                .id(SharedString::from(format!("bucket-tile-{name}")))
                .w(px(150.))
                .items_center()
                .gap(px(6.))
                .p(px(10.))
                .rounded(px(12.))
                .cursor_pointer()
                .hover(|s| s.bg(p.hover))
                .child(img("art/bucket.svg").size(px(84.)))
                .child(div().max_w(px(130.)).truncate().font_weight(FontWeight::MEDIUM).child(name.clone()))
                .child(div().text_size(px(11.)).text_color(p.ink_faint).child(created))
                .on_click(cx.listener(move |this, ev: &gpui_kit::ClickEvent, window, cx| {
                    if ev.click_count() >= 1 {
                        this.open_bucket(name.clone(), window, cx)
                    }
                }))
        });
        v_flex()
            .id("overview")
            .size_full()
            .overflow_y_scroll()
            .px(px(20.))
            .py(px(12.))
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(provider_badge(session.conn.provider, 40.))
                    .child(
                        v_flex()
                            .flex_1()
                            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(session.conn.name.clone()))
                            .child(div().text_color(p.ink_faint).child(format!(
                                "{} · {} bucket{}",
                                session.conn.provider.info().name,
                                session.buckets.len(),
                                if session.buckets.len() == 1 { "" } else { "s" }
                            ))),
                    )
                    .child(
                        Button::new("overview-new-bucket")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label("New bucket")
                            .on_click(cx.listener(|this, _, window, cx| this.create_bucket(window, cx))),
                    ),
            )
            .child(h_flex().flex_wrap().gap(px(6.)).children(tiles))
            .when(session.buckets.is_empty(), |d| {
                d.child(empty_state(IconName::Boxes, "No buckets yet", "Create one, or set a default bucket on the connection if your key can't list them.", p))
            })
            .into_any_element()
    }
}

// ---- tiles -----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn grid_tile_el(
    ix: usize,
    entry: &Entry,
    thumb_path: Option<std::path::PathBuf>,
    thumb: f32,
    tile_w: f32,
    is_sel: bool,
    size_label: String,
    view: &Entity<S3App>,
    p: &Palette,
) -> AnyElement {
    let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
    let (e1, key) = (entry.clone(), entry.key.clone());
    v_flex()
        .id(ix)
        .w(px(tile_w))
        .pt(px(6.))
        .items_center()
        .gap(px(3.))
        .child(
            div()
                .p(px(4.))
                .rounded(px(10.))
                .when(is_sel, |d| d.bg(p.selected))
                .child(file_visual(entry, thumb_path, thumb, false, p)),
        )
        .child(
            div()
                .max_w(px(tile_w - 10.))
                .mt(px(2.))
                .px(px(6.))
                .py(px(1.))
                .rounded(px(5.))
                .truncate()
                .text_size(px(12.))
                .when(is_sel, |d| d.bg(p.accent).text_color(p.on_accent))
                .child(entry.name.clone()),
        )
        .child(div().h(px(14.)).text_size(px(11.)).text_color(p.ink_faint).child(size_label))
        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
            v2.update(cx, |this, cx| this.ensure_selected(ix, &key, window, cx));
        })
        .on_click(move |event, window, cx| {
            let e = e1.clone();
            v1.update(cx, |this, cx| this.click_entry(ix, e, event, window, cx));
        })
        .context_menu(move |menu, _, cx| entry_menu(menu, v3.clone(), cx))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn media_tile_el(
    ix: usize,
    entry: &Entry,
    thumb_path: Option<std::path::PathBuf>,
    thumb: f32,
    tile_w: f32,
    is_sel: bool,
    view: &Entity<S3App>,
    p: &Palette,
) -> AnyElement {
    let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
    let (e1, key) = (entry.clone(), entry.key.clone());
    let has_thumb = thumb_path.is_some();
    div()
        .id(ix)
        .w(px(tile_w))
        .h(px(tile_w))
        .p(px(2.))
        .child(
            div()
                .relative()
                .size(px(thumb))
                .rounded(px(5.))
                .overflow_hidden()
                .bg(p.well)
                .when(is_sel, |d| d.border_2().border_color(p.accent))
                .child(file_visual(entry, thumb_path, thumb, true, p))
                .when(!has_thumb && thumb >= 90., |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom(px(4.))
                            .left(px(6.))
                            .right(px(6.))
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(p.ink_dull)
                            .child(entry.name.clone()),
                    )
                }),
        )
        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
            v2.update(cx, |this, cx| this.ensure_selected(ix, &key, window, cx));
        })
        .on_click(move |event, window, cx| {
            let e = e1.clone();
            v1.update(cx, |this, cx| this.click_entry(ix, e, event, window, cx));
        })
        .context_menu(move |menu, _, cx| entry_menu(menu, v3.clone(), cx))
        .into_any_element()
}

/// Right-click menu for files and folders.
pub fn entry_menu(menu: PopupMenu, view: Entity<S3App>, cx: &mut App) -> PopupMenu {
    let selected = view.read(cx).selected_entries(cx);
    let one = selected.len() == 1;
    let first = selected.first().cloned();
    let is_dir = first.as_ref().is_some_and(|e| e.is_dir);
    let single_file = one && !is_dir;
    let key = first.as_ref().map(|e| e.key.clone()).unwrap_or_default();
    let v = |_: ()| view.clone();
    let (v1, v2, v3, v4, v5, v6, v7, v8, v9, v10) = (v(()), v(()), v(()), v(()), v(()), v(()), v(()), v(()), v(()), v(()));
    let (k1, k2, k3) = (key.clone(), key.clone(), key.clone());
    let (f1, f2) = (first.clone(), first.clone());
    let folders: Vec<String> = selected.iter().filter(|e| e.is_dir).map(|e| e.key.clone()).collect();
    let v11 = view.clone();
    menu.when(one && is_dir, |m| {
        m.item(PopupMenuItem::new("Open").icon(IconName::FolderOpen).on_click(move |_, window, cx| {
            if let Some(e) = f1.clone() {
                v8.update(cx, |this, cx| this.open_entry(e, window, cx));
            }
        }))
    })
    .when(!folders.is_empty(), |m| {
        m.item(
            PopupMenuItem::new(if folders.len() > 1 { "Calculate sizes" } else { "Calculate size" })
                .icon(IconName::Gauge)
                .on_click(move |_, _, cx| {
                    let folders = folders.clone();
                    v11.update(cx, |this, cx| {
                        for f in folders {
                            this.calc_folder_size(f, true, cx);
                        }
                    })
                }),
        )
    })
    .when(single_file, |m| {
        let f3 = f2.clone();
        m.item(PopupMenuItem::new("Quick Look").icon(IconName::ScanEye).on_click(move |_, window, cx| {
            if let Some(e) = f2.clone() {
                v10.update(cx, |this, cx| this.open_preview(e, window, cx));
            }
        }))
        .item(PopupMenuItem::new("Open with default app").icon(IconName::SquareArrowOutUpRight).on_click(move |_, window, cx| {
            if let Some(e) = f3.clone() {
                v9.update(cx, |this, cx| this.open_external(e, window, cx));
            }
        }))
    })
    .item(
        PopupMenuItem::new(if selected.len() > 1 { format!("Download {} items…", selected.len()) } else { "Download…".to_string() })
            .icon(IconName::Download)
            .on_click(move |_, window, cx| v1.update(cx, |this, cx| this.download_selected(window, cx))),
    )
    .item(PopupMenuItem::new("Rename…").icon(IconName::Pencil).disabled(!one).on_click(move |_, window, cx| {
        v2.update(cx, |this, cx| this.rename_selected(window, cx))
    }))
    .separator()
    .item(PopupMenuItem::new("Copy key").icon(IconName::Copy).disabled(!one).on_click(move |_, window, cx| {
        v3.update(cx, |this, cx| this.copy_text(k1.clone(), "Key", window, cx))
    }))
    .item(PopupMenuItem::new("Copy S3 URI").icon(IconName::Copy).disabled(!one).on_click(move |_, window, cx| {
        v4.update(cx, |this, cx| {
            if let Some(uri) = this.s3_uri(&k2) {
                this.copy_text(uri, "S3 URI", window, cx);
            }
        })
    }))
    .item(PopupMenuItem::new("Copy share link (24 h)").icon(IconName::Link).disabled(!single_file).on_click(move |_, window, cx| {
        v5.update(cx, |this, cx| this.copy_presigned(k3.clone(), 86400, window, cx))
    }))
    .separator()
    .item(PopupMenuItem::new("Select all").on_click(move |_, _, cx| v6.update(cx, |this, cx| this.select_all(cx))))
    .item(PopupMenuItem::new("Delete…").icon(IconName::Trash).on_click(move |_, window, cx| {
        v7.update(cx, |this, cx| this.delete_selected(window, cx))
    }))
}
