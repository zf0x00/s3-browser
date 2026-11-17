//! App state and behaviour. Rendering lives in `crate::ui`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, ClipboardItem, Entity, FocusHandle, FontWeight, KeyDownEvent, PathPromptOptions, ScrollStrategy,
    SharedString, UniformListScrollHandle, Window, div, px,
};

use crate::cache;
use crate::connection_form::{ConnectionForm, FormStatus};
use crate::providers::ProviderKind;
use crate::s3::{self, BucketInfo, Entry, ObjectMeta, S3};
use crate::settings::{Settings, ViewMode};
use crate::store::{self, Connection};
use crate::theme;
use crate::transfers::{Direction, Transfer};
use crate::util;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Kind,
    Modified,
}

pub struct Session {
    pub conn: Connection,
    pub s3: S3,
    pub buckets: Vec<BucketInfo>,
    pub buckets_loading: bool,
    pub bucket: Option<String>,
    pub prefix: String,
    pub entries: Vec<Entry>,
    pub next_token: Option<String>,
    pub loading: bool,
    pub loading_more: bool,
    pub error: Option<String>,
    pub back: Vec<(Option<String>, String)>,
    pub forward: Vec<(Option<String>, String)>,
    generation: u64,
}

pub enum Detail {
    Loading(String),
    Ready(ObjectMeta),
    Failed(String, String),
}

#[derive(Clone)]
pub enum Thumb {
    Loading,
    Ready(PathBuf),
    Unavailable,
}

pub enum PreviewState {
    Loading,
    Ready(PathBuf),
    /// Not an image GPUI can draw full-size; show the thumbnail/icon.
    NoFullPreview,
    TooLarge,
    Failed(String),
}

pub struct Preview {
    pub key: String,
    pub state: PreviewState,
}

/// Largest image the Quick Preview fetches automatically.
const PREVIEW_FULL_MAX: u64 = 150 * 1024 * 1024;
/// Uploaded files up to this size are kept in the cache (with a thumbnail).
const SEED_ON_UPLOAD_MAX: u64 = 64 * 1024 * 1024;

pub struct S3App {
    pub connections: Vec<Connection>,
    pub session: Option<Session>,
    pub connecting: Option<String>,
    pub filter: Entity<InputState>,
    pub sort: SortKey,
    pub sort_asc: bool,
    pub selected: BTreeSet<String>,
    pub anchor: Option<usize>,
    pub detail: Option<Detail>,
    pub transfers: Vec<Arc<Transfer>>,
    pub show_transfers: bool,
    next_transfer_id: u64,
    limiter: Arc<tokio::sync::Semaphore>,
    ticking: bool,
    pub scroll: UniformListScrollHandle,
    pub settings: Settings,
    pub thumbs: HashMap<String, Thumb>,
    thumb_gen: Arc<AtomicU64>,
    thumb_limiter: Arc<tokio::sync::Semaphore>,
    pub preview: Option<Preview>,
    /// The in-flight full-size fetch for Quick Look (cancelled on step/close).
    preview_fetch: Option<Arc<Transfer>>,
    pub focus: FocusHandle,
    /// Columns in the last grid render (for arrow-key navigation).
    pub grid_cols: usize,
    pub switcher_open: bool,
    /// Folder sizes by `bucket/prefix` (computed on demand by listing).
    pub folder_sizes: HashMap<String, FolderSize>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum FolderSize {
    Counting,
    Done(s3::PrefixSize),
    Failed,
}

/// Selecting a folder counts at most this many objects automatically;
/// "Calculate size" counts everything.
const AUTO_SIZE_CAP: u64 = 50_000;

pub fn folder_key(bucket: &str, prefix: &str) -> String {
    format!("{bucket}/{prefix}")
}

/// Thumbnail map key: one entry per object version.
pub fn thumb_key(e: &Entry) -> String {
    format!("{}#{}#{}", e.key, e.etag.as_deref().unwrap_or(""), e.size)
}

impl S3App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search this folder"));
        cx.subscribe_in(&filter, window, |this: &mut Self, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.anchor = None;
                cx.notify();
            }
        })
        .detach();
        let settings = Settings::load();
        cache::global().set_limit(settings.cache_bytes());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            connections: store::load(),
            session: None,
            connecting: None,
            filter,
            sort: SortKey::Name,
            sort_asc: true,
            selected: BTreeSet::new(),
            anchor: None,
            detail: None,
            transfers: Vec::new(),
            show_transfers: false,
            next_transfer_id: 1,
            limiter: Arc::new(tokio::sync::Semaphore::new(4)),
            ticking: false,
            scroll: UniformListScrollHandle::new(),
            settings,
            thumbs: HashMap::new(),
            thumb_gen: Arc::new(AtomicU64::new(0)),
            thumb_limiter: Arc::new(tokio::sync::Semaphore::new(4)),
            preview: None,
            preview_fetch: None,
            focus,
            grid_cols: 1,
            switcher_open: false,
            folder_sizes: HashMap::new(),
        }
    }

    // ---- small helpers -------------------------------------------------------

    pub fn notify_err(window: &mut Window, cx: &mut App, msg: impl Into<SharedString>) {
        window.push_notification(Notification::error(msg), cx);
    }

    pub fn notify_ok(window: &mut Window, cx: &mut App, msg: impl Into<SharedString>) {
        window.push_notification(Notification::success(msg), cx);
    }

    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.save();
        cache::global().set_limit(self.settings.cache_bytes());
        cx.notify();
    }

    /// The entries shown: filtered, folders first, then sorted.
    pub fn visible(&self, cx: &App) -> Vec<Entry> {
        let Some(session) = &self.session else { return Vec::new() };
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let media = self.settings.view == ViewMode::Media;
        let mut out: Vec<Entry> = session
            .entries
            .iter()
            .filter(|e| needle.is_empty() || e.name.to_lowercase().contains(&needle))
            .filter(|e| !media || e.is_dir || cache::is_previewable(&e.name) || util::is_video(&e.name))
            .cloned()
            .collect();
        let (key, asc) = (self.sort, self.sort_asc);
        out.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                let ord = match key {
                    SortKey::Name => natural_cmp(&a.name, &b.name),
                    SortKey::Size => a.size.cmp(&b.size),
                    SortKey::Kind => util::kind_label(&a.name).cmp(&util::kind_label(&b.name)),
                    SortKey::Modified => a.modified.cmp(&b.modified),
                };
                if asc { ord } else { ord.reverse() }
            })
        });
        out
    }

    pub fn selected_entries(&self, cx: &App) -> Vec<Entry> {
        self.visible(cx).into_iter().filter(|e| self.selected.contains(&e.key)).collect()
    }

    pub fn location(&self) -> Option<(S3, String, String)> {
        let s = self.session.as_ref()?;
        Some((s.s3.clone(), s.bucket.clone()?, s.prefix.clone()))
    }

    pub fn is_cached(&self, entry: &Entry) -> bool {
        let Some((s3c, bucket, _)) = self.location() else { return false };
        !entry.is_dir && cache::global().is_cached(&s3c.object_ref(&bucket, entry))
    }

    // ---- connections ---------------------------------------------------------

    fn persist(&self, window: &mut Window, cx: &mut App) {
        if let Err(e) = store::save(&self.connections) {
            Self::notify_err(window, cx, format!("Couldn't save connections: {e}"));
        }
    }

    pub fn open_connection_dialog(
        &mut self,
        existing: Option<Connection>,
        provider: Option<ProviderKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = existing.is_some();
        let form = cx.new(|cx| ConnectionForm::new(existing, provider, window, cx));
        let app = cx.entity();
        let first_draw = std::cell::Cell::new(true);

        let save: Rc<dyn Fn(bool, &mut Window, &mut App)> = {
            let (form, app) = (form.clone(), app.clone());
            Rc::new(move |connect: bool, window: &mut Window, cx: &mut App| {
                let built = form.read(cx).build(cx);
                match built {
                    Err(e) => form.update(cx, |f, cx| {
                        f.status = FormStatus::Err(e.to_string());
                        cx.notify();
                    }),
                    Ok((conn, secret)) => {
                        if let Some(secret) = &secret {
                            if let Err(e) = store::save_secret(&conn.id, secret) {
                                Self::notify_err(window, cx, format!("Couldn't save the secret key: {e}"));
                                return;
                            }
                        }
                        let id = conn.id.clone();
                        app.update(cx, |this, cx| {
                            match this.connections.iter_mut().find(|c| c.id == conn.id) {
                                Some(existing) => *existing = conn.clone(),
                                None => this.connections.push(conn.clone()),
                            }
                            this.persist(window, cx);
                            let active = this.session.as_ref().is_some_and(|s| s.conn.id == id);
                            if connect || active {
                                this.connect(&id, window, cx);
                            }
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                }
            })
        };

        window.open_dialog(cx, move |dialog, window, cx| {
            if first_draw.replace(false) {
                form.update(cx, |f, cx| f.focus_first(window, cx));
            }
            let imports_form = form.clone();
            let test_form = form.clone();
            let (save_only, save_connect) = (save.clone(), save.clone());
            dialog
                .w(px(660.))
                .title(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if editing { "Edit connection" } else { "New connection" }),
                )
                .child(form.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .gap(px(8.))
                        .child(
                            Button::new("import-creds")
                                .ghost()
                                .icon(IconName::Import)
                                .label("Import…")
                                .tooltip("~/.aws/credentials or ~/.passwd-s3fs")
                                .dropdown_menu(move |menu, _, _| {
                                    let found = store::import_all();
                                    if found.is_empty() {
                                        return menu.item(
                                            PopupMenuItem::new("No credentials in ~/.aws/credentials or ~/.passwd-s3fs").disabled(true),
                                        );
                                    }
                                    let mut menu = menu;
                                    for cred in found {
                                        let f = imports_form.clone();
                                        menu = menu.item(PopupMenuItem::new(cred.label.clone()).icon(IconName::KeyRound).on_click(
                                            move |_, window, cx| f.update(cx, |f, cx| f.apply_import(&cred, window, cx)),
                                        ));
                                    }
                                    menu
                                }),
                        )
                        .child(
                            Button::new("test-conn")
                                .outline()
                                .icon(IconName::Plug)
                                .label("Test")
                                .on_click(move |_, _, cx| test_form.update(cx, |f, cx| f.test(cx))),
                        )
                        .child(div().flex_1())
                        .child(Button::new("conn-cancel").ghost().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                        .child(Button::new("conn-save").outline().label("Save").on_click(move |_, window, cx| save_only(false, window, cx)))
                        .child(
                            Button::new("conn-save-connect")
                                .primary()
                                .label("Save & Connect")
                                .on_click(move |_, window, cx| save_connect(true, window, cx)),
                        ),
                )
        });
    }

    pub fn delete_connection(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() else { return };
        let app = cx.entity();
        self.confirm(
            "Remove connection?",
            format!("“{}” and its saved secret key will be removed from this Mac. Nothing is deleted from the storage.", conn.name),
            "Remove",
            window,
            cx,
            move |window, cx| {
                app.update(cx, |this, cx| {
                    this.connections.retain(|c| c.id != id);
                    store::delete_secret(&id);
                    if this.session.as_ref().is_some_and(|s| s.conn.id == id) {
                        this.disconnect(cx);
                    }
                    this.persist(window, cx);
                    cx.notify();
                });
            },
        );
    }

    pub fn duplicate_connection(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut conn) = self.connections.iter().find(|c| c.id == id).cloned() else { return };
        let old = conn.id.clone();
        conn.id = store::new_id();
        conn.name = format!("{} copy", conn.name);
        if let Ok(secret) = store::load_secret(&old) {
            let _ = store::save_secret(&conn.id, &secret);
        }
        self.connections.push(conn);
        self.persist(window, cx);
        cx.notify();
    }

    pub fn connect(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() else { return };
        self.connecting = Some(conn.id.clone());
        self.switcher_open = false;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = s3::spawn(async move {
                let id = conn.id.clone();
                let secret = tokio::task::spawn_blocking(move || store::load_secret(&id)).await??;
                let client = S3::new(&conn, &secret);
                let buckets = client.list_buckets().await?;
                Ok((client, conn, buckets))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.connecting = None;
                match result {
                    Ok((client, conn, buckets)) => {
                        let auto = conn.bucket.clone().filter(|b| !b.is_empty()).or_else(|| {
                            if buckets.len() == 1 { Some(buckets[0].name.clone()) } else { None }
                        });
                        this.session = Some(Session {
                            conn,
                            s3: client,
                            buckets,
                            buckets_loading: false,
                            bucket: None,
                            prefix: String::new(),
                            entries: Vec::new(),
                            next_token: None,
                            loading: false,
                            loading_more: false,
                            error: None,
                            back: Vec::new(),
                            forward: Vec::new(),
                            generation: 0,
                        });
                        this.thumbs.clear();
                        this.clear_selection();
                        if let Some(b) = auto {
                            this.open_bucket(b, window, cx);
                        }
                    }
                    Err(e) => Self::notify_err(window, cx, format!("Couldn't connect: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        self.preview = None;
        self.thumbs.clear();
        self.thumb_gen.fetch_add(1, Ordering::Relaxed);
        self.clear_selection();
        cx.notify();
    }

    pub fn refresh_buckets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else { return };
        session.buckets_loading = true;
        let s3c = session.s3.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = s3::spawn(async move { s3c.list_buckets().await }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(session) = this.session.as_mut() {
                    session.buckets_loading = false;
                    match result {
                        Ok(b) => session.buckets = b,
                        Err(e) => Self::notify_err(window, cx, format!("Couldn't list buckets: {e}")),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- navigation ----------------------------------------------------------

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.detail = None;
    }

    fn push_history(&mut self) {
        if let Some(s) = self.session.as_mut() {
            s.back.push((s.bucket.clone(), s.prefix.clone()));
            s.forward.clear();
        }
    }

    /// `None` shows the buckets overview.
    pub fn open_bucket_or_home(&mut self, bucket: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return };
        if session.bucket == bucket && session.prefix.is_empty() {
            return;
        }
        self.push_history();
        let session = self.session.as_mut().unwrap();
        session.bucket = bucket;
        session.prefix = String::new();
        self.after_move(window, cx);
    }

    pub fn open_bucket(&mut self, bucket: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_bucket_or_home(Some(bucket), window, cx);
    }

    pub fn navigate(&mut self, prefix: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return };
        if session.bucket.is_none() || session.prefix == prefix {
            return;
        }
        self.push_history();
        self.session.as_mut().unwrap().prefix = prefix;
        self.after_move(window, cx);
    }

    fn after_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        // Thumbnails queued for the folder we left aren't needed any more.
        self.thumb_gen.fetch_add(1, Ordering::Relaxed);
        self.thumbs.retain(|_, t| !matches!(t, Thumb::Loading));
        let has_bucket = self.session.as_ref().is_some_and(|s| s.bucket.is_some());
        if has_bucket {
            self.reload(window, cx);
        } else {
            if let Some(s) = self.session.as_mut() {
                s.entries.clear();
                s.next_token = None;
                s.error = None;
            }
            self.clear_selection();
            cx.notify();
        }
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else { return };
        let Some((bucket, prefix)) = session.back.pop() else { return };
        session.forward.push((session.bucket.clone(), session.prefix.clone()));
        session.bucket = bucket;
        session.prefix = prefix;
        self.after_move(window, cx);
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else { return };
        let Some((bucket, prefix)) = session.forward.pop() else { return };
        session.back.push((session.bucket.clone(), session.prefix.clone()));
        session.bucket = bucket;
        session.prefix = prefix;
        self.after_move(window, cx);
    }

    pub fn go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return };
        if session.prefix.is_empty() {
            return;
        }
        let trimmed = session.prefix.trim_end_matches('/');
        let parent = match trimmed.rfind('/') {
            Some(i) => trimmed[..=i].to_string(),
            None => String::new(),
        };
        self.navigate(parent, window, cx);
    }

    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_selection();
        self.load_page(false, window, cx);
    }

    pub fn load_page(&mut self, more: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else { return };
        let Some(bucket) = session.bucket.clone() else { return };
        session.generation += 1;
        let generation = session.generation;
        let token = if more { session.next_token.clone() } else { None };
        if more {
            session.loading_more = true;
        } else {
            session.loading = true;
            session.error = None;
        }
        let (s3c, prefix) = (session.s3.clone(), session.prefix.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = s3::spawn(async move { s3c.list(&bucket, &prefix, token).await }).await;
            let _ = this.update_in(cx, |this, _, cx| {
                let Some(session) = this.session.as_mut() else { return };
                if session.generation != generation {
                    return;
                }
                session.loading = false;
                session.loading_more = false;
                match result {
                    Ok(listing) => {
                        if more {
                            session.entries.extend(listing.entries);
                        } else {
                            session.entries = listing.entries;
                            this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        }
                        session.next_token = listing.next_token;
                    }
                    Err(e) => {
                        if !more {
                            session.entries.clear();
                        }
                        session.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- selection & keyboard ------------------------------------------------

    pub fn click_entry(&mut self, ix: usize, entry: Entry, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if event.click_count() >= 2 {
            self.open_entry(entry, window, cx);
            return;
        }
        let m = event.modifiers();
        if m.shift {
            let visible = self.visible(cx);
            if visible.is_empty() {
                return;
            }
            let last = visible.len() - 1;
            let from = self.anchor.unwrap_or(ix).min(last);
            let (a, b) = if from <= ix { (from, ix) } else { (ix, from) };
            if !m.platform {
                self.selected.clear();
            }
            for e in &visible[a..=b.min(last)] {
                self.selected.insert(e.key.clone());
            }
        } else if m.platform || m.control {
            if !self.selected.remove(&entry.key) {
                self.selected.insert(entry.key.clone());
            }
            self.anchor = Some(ix);
        } else {
            self.selected.clear();
            self.selected.insert(entry.key.clone());
            self.anchor = Some(ix);
        }
        self.refresh_detail(window, cx);
        cx.notify();
    }

    pub fn ensure_selected(&mut self, ix: usize, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.contains(key) {
            self.selected.clear();
            self.selected.insert(key.to_string());
            self.anchor = Some(ix);
            self.refresh_detail(window, cx);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = self.visible(cx).into_iter().map(|e| e.key).collect();
        self.detail = None;
        cx.notify();
    }

    fn select_index(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        let Some(e) = visible.get(ix) else { return };
        self.selected = BTreeSet::from([e.key.clone()]);
        self.anchor = Some(ix);
        let cols = if self.settings.view == ViewMode::List { 1 } else { self.grid_cols.max(1) };
        self.scroll.scroll_to_item(ix / cols, ScrollStrategy::Center);
        self.refresh_detail(window, cx);
        cx.notify();
    }

    pub fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        let cmd = ks.modifiers.platform;

        if self.preview.is_some() {
            match key {
                "escape" | "space" => self.close_preview(cx),
                "left" | "up" => self.step_preview(-1, window, cx),
                "right" | "down" => self.step_preview(1, window, cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        // Typing in the search box etc. shouldn't trigger shortcuts.
        if !self.focus.is_focused(window) {
            return;
        }
        let visible_len = self.visible(cx).len();
        let current = self.anchor.filter(|i| *i < visible_len);
        let cols = if self.settings.view == ViewMode::List { 1 } else { self.grid_cols.max(1) };
        match (key, cmd) {
            ("space", false) => {
                if let [entry] = self.selected_entries(cx).as_slice() {
                    if !entry.is_dir {
                        self.open_preview(entry.clone(), window, cx);
                    }
                }
            }
            ("enter", false) => {
                if let [entry] = self.selected_entries(cx).as_slice() {
                    self.open_entry(entry.clone(), window, cx);
                }
            }
            ("escape", _) => {
                self.clear_selection();
                cx.notify();
            }
            ("a", true) => self.select_all(cx),
            ("backspace" | "delete", true) => self.delete_selected(window, cx),
            ("up", true) => self.go_up(window, cx),
            ("[", true) => self.go_back(window, cx),
            ("]", true) => self.go_forward(window, cx),
            ("r", true) => self.reload(window, cx),
            ("left", false) if visible_len > 0 => self.select_index(current.map_or(0, |i| i.saturating_sub(1)), window, cx),
            ("right", false) if visible_len > 0 => {
                self.select_index(current.map_or(0, |i| (i + 1).min(visible_len - 1)), window, cx)
            }
            ("up", false) if visible_len > 0 => self.select_index(current.map_or(0, |i| i.saturating_sub(cols)), window, cx),
            ("down", false) if visible_len > 0 => {
                self.select_index(current.map_or(0, |i| (i + cols).min(visible_len - 1)), window, cx)
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    pub fn refresh_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.selected_entries(cx);
        let [entry] = entries.as_slice() else {
            self.detail = None;
            return;
        };
        if entry.is_dir {
            self.detail = None;
            let key = entry.key.clone();
            self.calc_folder_size(key, false, cx);
            return;
        }
        let Some((s3c, bucket, _)) = self.location() else { return };
        let key = entry.key.clone();
        if matches!(&self.detail, Some(Detail::Ready(m)) if m.key == key) {
            return;
        }
        self.detail = Some(Detail::Loading(key.clone()));
        cx.spawn_in(window, async move |this, cx| {
            let k = key.clone();
            let result = s3::spawn(async move { s3c.head(&bucket, &k).await }).await;
            let _ = this.update(cx, |this, cx| {
                if matches!(&this.detail, Some(Detail::Loading(k)) if *k == key) {
                    this.detail = Some(match result {
                        Ok(meta) => Detail::Ready(meta),
                        Err(e) => Detail::Failed(key, e.to_string()),
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Double-click / Enter: folders open, images preview, other files open
    /// in their default app (from the cache).
    pub fn open_entry(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.navigate(entry.key, window, cx);
        } else if cache::is_previewable(&entry.name) || cache::is_viewable_image(&entry.name) {
            self.open_preview(entry, window, cx);
        } else {
            self.open_external(entry, window, cx);
        }
    }

    /// Clear "no thumbnail" marks so new preview settings take effect.
    pub fn retry_thumbnails(&mut self, cx: &mut Context<Self>) {
        self.thumbs.retain(|_, t| !matches!(t, Thumb::Unavailable));
        cx.notify();
    }

    // ---- thumbnails ----------------------------------------------------------

    /// Queue thumbnails for entries that just became visible.
    pub fn request_thumbs(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        if !self.settings.previews {
            return;
        }
        let Some((s3c, bucket, _)) = self.location() else { return };
        let max = self.settings.preview_max_bytes();
        for entry in entries {
            let tk = thumb_key(&entry);
            if entry.is_dir || self.thumbs.contains_key(&tk) || !cache::is_previewable(&entry.name) {
                continue;
            }
            self.thumbs.insert(tk.clone(), Thumb::Loading);
            let (s3c, bucket, current_gen, limiter) = (s3c.clone(), bucket.clone(), self.thumb_gen.clone(), self.thumb_limiter.clone());
            let my_gen = current_gen.load(Ordering::Relaxed);
            cx.spawn(async move |this, cx| {
                let result = s3::spawn(async move {
                    let _permit = limiter.acquire_owned().await.map_err(|e| anyhow!("{e}"))?;
                    if current_gen.load(Ordering::Relaxed) != my_gen {
                        bail!("stale");
                    }
                    s3c.thumbnail(&bucket, &entry, max).await
                })
                .await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(path) => {
                            this.thumbs.insert(tk, Thumb::Ready(path));
                        }
                        Err(e) if e.to_string() == "stale" => {
                            this.thumbs.remove(&tk);
                        }
                        Err(_) => {
                            this.thumbs.insert(tk, Thumb::Unavailable);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub fn thumb_for(&self, entry: &Entry) -> Option<PathBuf> {
        match self.thumbs.get(&thumb_key(entry)) {
            Some(Thumb::Ready(p)) => Some(p.clone()),
            _ => None,
        }
    }

    // ---- quick preview -------------------------------------------------------

    pub fn close_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.preview_fetch.take() {
            t.cancel();
        }
        self.preview = None;
        cx.notify();
    }

    pub fn open_preview(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.preview_fetch.take() {
            t.cancel(); // stepping on: stop paying for the previous image
        }
        window.focus(&self.focus, cx);
        self.selected = BTreeSet::from([entry.key.clone()]);
        self.refresh_detail(window, cx);
        let key = entry.key.clone();
        if !cache::is_viewable_image(&entry.name) {
            self.request_thumbs(vec![entry], cx);
            self.preview = Some(Preview { key, state: PreviewState::NoFullPreview });
            cx.notify();
            return;
        }
        if entry.size > PREVIEW_FULL_MAX {
            self.request_thumbs(vec![entry], cx);
            self.preview = Some(Preview { key, state: PreviewState::TooLarge });
            cx.notify();
            return;
        }
        let Some((s3c, bucket, _)) = self.location() else { return };
        self.preview = Some(Preview { key: key.clone(), state: PreviewState::Loading });
        cx.notify();
        let t = self.new_transfer(Direction::Download, entry.name.clone(), format!("{bucket}/{}", entry.key), Path::new(""));
        self.preview_fetch = Some(t.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = s3::spawn(async move { s3c.ensure_cached(&bucket, &entry, t).await }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = this.preview.as_mut().filter(|p| p.key == key) {
                    p.state = match result {
                        Ok(path) => PreviewState::Ready(path),
                        Err(e) => PreviewState::Failed(e.to_string()),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Move the preview to the previous/next file in the folder.
    pub fn step_preview(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.preview.as_ref().map(|p| p.key.clone()) else { return };
        let files: Vec<Entry> = self.visible(cx).into_iter().filter(|e| !e.is_dir).collect();
        let Some(i) = files.iter().position(|e| e.key == current) else { return };
        let next = (i as i64 + delta).rem_euclid(files.len() as i64) as usize;
        let entry = files[next].clone();
        if let Some(ix) = self.visible(cx).iter().position(|e| e.key == entry.key) {
            self.anchor = Some(ix);
            let cols = if self.settings.view == ViewMode::List { 1 } else { self.grid_cols.max(1) };
            self.scroll.scroll_to_item(ix / cols, ScrollStrategy::Center);
        }
        self.open_preview(entry, window, cx);
    }

    /// Open with the default app, from the cache (fetched once if needed).
    /// Large files ask first; files too big for the cache go to a temp folder.
    pub fn open_external(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        const ASK_ABOVE: u64 = 100 * 1024 * 1024;
        let cached = self.is_cached(&entry);
        if !cached && entry.size > ASK_ABOVE {
            let app = cx.entity();
            let e = entry.clone();
            self.confirm(
                "Download to open?",
                format!("“{}” is {} and isn't cached yet. Opening it downloads the whole file.", entry.name, util::human_size(entry.size)),
                "Download & open",
                window,
                cx,
                move |window, cx| {
                    let e = e.clone();
                    app.update(cx, |this, cx| this.open_external_now(e, window, cx));
                },
            );
            return;
        }
        self.open_external_now(entry, window, cx);
    }

    fn open_external_now(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let Some((s3c, bucket, _)) = self.location() else { return };
        let t = self.new_transfer(Direction::Download, entry.name.clone(), format!("{bucket}/{}", entry.key), Path::new("cache"));
        let tt = t.clone();
        let too_big = entry.size > cache::global().max_object();
        let handle = s3::spawn(async move {
            if too_big {
                // Bigger than the cache allows: a one-off temp copy.
                let dir = std::env::temp_dir().join("S3 Browser Open").join(uuid::Uuid::new_v4().simple().to_string());
                let dest = dir.join(&entry.name);
                s3c.download_file(&bucket, &entry.key, &dest, tt).await?;
                cache::global().record_fetched(entry.size);
                Ok(dest)
            } else {
                s3c.ensure_cached(&bucket, &entry, tt).await
            }
        });
        self.track(t.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = handle.await;
            t.finish(&result.as_ref().map(|_| ()).map_err(|e| anyhow!("{e}")));
            let _ = this.update_in(cx, |_, window, cx| match result {
                Ok(path) => cx.open_with_system(&path),
                Err(e) => Self::notify_err(window, cx, format!("Couldn't open {}: {e}", t.name)),
            });
        })
        .detach();
    }

    // ---- dialogs -------------------------------------------------------------

    pub fn confirm(
        &mut self,
        title: &'static str,
        message: String,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_ok: impl Fn(&mut Window, &mut App) + 'static,
    ) {
        let on_ok = Rc::new(on_ok);
        window.open_dialog(cx, move |dialog, _, cx| {
            let p = theme::palette(cx);
            let ok = on_ok.clone();
            dialog
                .w(px(440.))
                .title(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_sm().text_color(p.ink_dull).child(message.clone()))
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap(px(8.))
                        .child(Button::new("confirm-cancel").ghost().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                        .child(Button::new("confirm-ok").danger().label(action).on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            ok(window, cx);
                        })),
                )
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prompt(
        &mut self,
        title: &'static str,
        label: &'static str,
        initial: String,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_submit: impl Fn(String, &mut Window, &mut App) + 'static,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let first_draw = std::cell::Cell::new(true);
        let submit: Rc<dyn Fn(&mut Window, &mut App)> = {
            let input = input.clone();
            Rc::new(move |window, cx| {
                let value = input.read(cx).value().trim().to_string();
                if value.is_empty() {
                    return;
                }
                window.close_dialog(cx);
                on_submit(value, window, cx);
            })
        };
        let on_enter = submit.clone();
        cx.subscribe_in(&input, window, move |_, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                // We're inside an update of S3App; `on_submit` updates it again.
                let on_enter = on_enter.clone();
                window.defer(cx, move |window, cx| on_enter(window, cx));
            }
        })
        .detach();
        window.open_dialog(cx, move |dialog, window, cx| {
            if first_draw.replace(false) {
                input.update(cx, |s, cx| s.focus(window, cx));
            }
            let p = theme::palette(cx);
            let go = submit.clone();
            dialog
                .w(px(440.))
                .title(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(
                    v_flex()
                        .gap(px(6.))
                        .child(div().text_xs().text_color(p.ink_faint).child(label))
                        .child(Input::new(&input)),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap(px(8.))
                        .child(Button::new("prompt-cancel").ghost().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                        .child(Button::new("prompt-ok").primary().label(action).on_click(move |_, window, cx| go(window, cx))),
                )
        });
    }

    // ---- object operations ---------------------------------------------------

    fn mutate<T: Send + 'static>(
        &mut self,
        work: impl std::future::Future<Output = Result<T>> + Send + 'static,
        done: impl FnOnce(T) -> String + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session.as_mut() {
            session.loading = true;
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = s3::spawn(work).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(v) => {
                        let msg = done(v);
                        if !msg.is_empty() {
                            Self::notify_ok(window, cx, msg);
                        }
                    }
                    Err(e) => Self::notify_err(window, cx, e.to_string()),
                }
                // Sizes may have changed anywhere above what we touched.
                this.folder_sizes.clear();
                this.reload(window, cx);
            });
        })
        .detach();
    }

    pub fn folder_size(&self, prefix: &str) -> Option<FolderSize> {
        let (_, bucket, _) = self.location()?;
        self.folder_sizes.get(&folder_key(&bucket, prefix)).copied()
    }

    /// Count a folder's size. `full` ignores the automatic object cap
    /// (and recounts a capped result).
    pub fn calc_folder_size(&mut self, prefix: String, full: bool, cx: &mut Context<Self>) {
        let Some((s3c, bucket, _)) = self.location() else { return };
        let fk = folder_key(&bucket, &prefix);
        match self.folder_sizes.get(&fk) {
            Some(FolderSize::Counting) => return,
            Some(FolderSize::Done(s)) if s.complete || !full => return,
            _ => {}
        }
        self.folder_sizes.insert(fk.clone(), FolderSize::Counting);
        cx.notify();
        let cap = if full { None } else { Some(AUTO_SIZE_CAP) };
        cx.spawn(async move |this, cx| {
            let result = s3::spawn(async move { s3c.prefix_size(&bucket, &prefix, cap).await }).await;
            let _ = this.update(cx, |this, cx| {
                this.folder_sizes.insert(fk, match result {
                    Ok(size) => FolderSize::Done(size),
                    Err(_) => FolderSize::Failed,
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((s3c, bucket, prefix)) = self.location() else { return };
        let app = cx.entity();
        self.prompt("New folder", "Folder name", String::new(), "Create", window, cx, move |name, window, cx| {
            let key = format!("{prefix}{}/", name.trim_matches('/'));
            let (s3c, bucket) = (s3c.clone(), bucket.clone());
            app.update(cx, |this, cx| {
                this.mutate(async move { s3c.create_folder(&bucket, &key).await }, move |_| format!("Created “{name}”"), window, cx)
            });
        });
    }

    pub fn rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.selected_entries(cx);
        let [entry] = entries.as_slice() else { return };
        let Some((s3c, bucket, prefix)) = self.location() else { return };
        let entry = entry.clone();
        let app = cx.entity();
        self.prompt("Rename", "New name", entry.name.clone(), "Rename", window, cx, move |name, window, cx| {
            let name = name.trim_matches('/').to_string();
            if name == entry.name {
                return;
            }
            let to = if entry.is_dir { format!("{prefix}{name}/") } else { format!("{prefix}{name}") };
            let (s3c, bucket, from, is_dir) = (s3c.clone(), bucket.clone(), entry.key.clone(), entry.is_dir);
            app.update(cx, |this, cx| {
                this.mutate(
                    async move { s3c.rename(&bucket, &from, &to, is_dir).await },
                    move |n| if is_dir { format!("Renamed folder ({n} objects moved)") } else { format!("Renamed to “{name}”") },
                    window,
                    cx,
                )
            });
        });
    }

    pub fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.selected_entries(cx);
        if entries.is_empty() {
            return;
        }
        let Some((s3c, bucket, _)) = self.location() else { return };
        let folders = entries.iter().filter(|e| e.is_dir).count();
        let what = match (entries.len(), folders) {
            (1, 1) => format!("the folder “{}” and everything in it", entries[0].name),
            (1, _) => format!("“{}”", entries[0].name),
            (n, 0) => format!("{n} files"),
            (n, f) => format!("{n} items ({f} folders, with everything in them)"),
        };
        let app = cx.entity();
        let targets: Vec<(String, bool)> = entries.iter().map(|e| (e.key.clone(), e.is_dir)).collect();
        self.confirm(
            "Delete permanently?",
            format!("This deletes {what} from “{bucket}”. S3 has no trash — this can't be undone (unless versioning is on)."),
            "Delete",
            window,
            cx,
            move |window, cx| {
                let (s3c, bucket, targets) = (s3c.clone(), bucket.clone(), targets.clone());
                app.update(cx, |this, cx| {
                    this.mutate(
                        async move { s3c.delete_entries(&bucket, targets).await },
                        |n| format!("Deleted {n} object{}", if n == 1 { "" } else { "s" }),
                        window,
                        cx,
                    )
                });
            },
        );
    }

    pub fn create_bucket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return };
        let s3c = session.s3.clone();
        let app = cx.entity();
        self.prompt("New bucket", "Bucket name (lowercase, 3–63 chars)", String::new(), "Create", window, cx, move |name, window, cx| {
            if !util::valid_bucket_name(&name) {
                Self::notify_err(window, cx, "Bucket names use 3–63 lowercase letters, digits, dots or hyphens.");
                return;
            }
            let s3c = s3c.clone();
            app.update(cx, |_, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    let n = name.clone();
                    let result = s3::spawn(async move { s3c.create_bucket(&n).await }).await;
                    let _ = this.update_in(cx, |this, window, cx| match result {
                        Ok(()) => {
                            Self::notify_ok(window, cx, format!("Created bucket “{name}”"));
                            this.refresh_buckets(window, cx);
                            this.open_bucket(name, window, cx);
                        }
                        Err(e) => Self::notify_err(window, cx, format!("Couldn't create the bucket: {e}")),
                    });
                })
                .detach();
            });
        });
    }

    pub fn delete_bucket(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return };
        let s3c = session.s3.clone();
        let app = cx.entity();
        self.confirm(
            "Delete bucket?",
            format!("Delete the bucket “{name}”? Most providers only allow deleting empty buckets."),
            "Delete bucket",
            window,
            cx,
            move |window, cx| {
                let (s3c, name) = (s3c.clone(), name.clone());
                app.update(cx, |_, cx| {
                    cx.spawn_in(window, async move |this, cx| {
                        let n = name.clone();
                        let result = s3::spawn(async move { s3c.delete_bucket(&n).await }).await;
                        let _ = this.update_in(cx, |this, window, cx| match result {
                            Ok(()) => {
                                if this.session.as_ref().is_some_and(|s| s.bucket.as_deref() == Some(name.as_str())) {
                                    this.open_bucket_or_home(None, window, cx);
                                }
                                Self::notify_ok(window, cx, format!("Deleted bucket “{name}”"));
                                this.refresh_buckets(window, cx);
                            }
                            Err(e) => Self::notify_err(window, cx, format!("Couldn't delete the bucket: {e}")),
                        });
                    })
                    .detach();
                });
            },
        );
    }

    pub fn s3_uri(&self, key: &str) -> Option<String> {
        let (_, bucket, _) = self.location()?;
        Some(format!("s3://{bucket}/{key}"))
    }

    pub fn copy_text(&self, text: String, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        Self::notify_ok(window, cx, format!("{what} copied"));
    }

    pub fn copy_presigned(&mut self, key: String, secs: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((s3c, bucket, _)) = self.location() else { return };
        cx.spawn_in(window, async move |_, cx| {
            let result = s3::spawn(async move { s3c.presign_get(&bucket, &key, Duration::from_secs(secs)).await }).await;
            let _ = cx.update(|window, cx| match result {
                Ok(url) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    let label = match secs {
                        3600 => "1 hour",
                        86400 => "24 hours",
                        _ => "7 days",
                    };
                    Self::notify_ok(window, cx, format!("Share link copied (valid {label})"));
                }
                Err(e) => Self::notify_err(window, cx, format!("Couldn't sign the link: {e}")),
            });
        })
        .detach();
    }

    // ---- transfers -----------------------------------------------------------

    fn new_transfer(&mut self, direction: Direction, name: String, remote: String, local: &Path) -> Arc<Transfer> {
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;
        Transfer::new(id, direction, name, remote, local.display().to_string())
    }

    /// Show a transfer that runs elsewhere (open in default app) in the list.
    fn track(&mut self, t: Arc<Transfer>, cx: &mut Context<Self>) {
        t.set_running();
        self.transfers.insert(0, t);
        self.ensure_ticking(cx);
    }

    fn start_transfer(
        &mut self,
        t: Arc<Transfer>,
        job: impl std::future::Future<Output = Result<()>> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfers.insert(0, t.clone());
        self.show_transfers = true;
        let limiter = self.limiter.clone();
        let task_t = t.clone();
        let handle = s3::spawn(async move {
            let _permit = limiter.acquire_owned().await.map_err(|e| anyhow!("{e}"))?;
            task_t.check_cancel()?;
            task_t.set_running();
            let result = job.await;
            task_t.finish(&result);
            result
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = handle.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = &result {
                    if !t.is_cancelled() {
                        Self::notify_err(window, cx, format!("{}: {e}", t.name));
                    }
                }
                if t.direction == Direction::Upload
                    && !this.transfers.iter().any(|x| x.direction == Direction::Upload && x.is_active())
                {
                    this.folder_sizes.clear();
                    this.load_page(false, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        self.ensure_ticking(cx);
    }

    fn ensure_ticking(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        let active = this.transfers.iter().any(|t| t.is_active());
                        if !active {
                            this.ticking = false;
                        }
                        active
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn upload_dialog(&mut self, folders: bool, window: &mut Window, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: !folders,
            directories: folders,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = answer.await else { return };
            let _ = this.update_in(cx, |this, window, cx| this.upload_paths(paths, window, cx));
        })
        .detach();
    }

    pub fn upload_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((s3c, bucket, prefix)) = self.location() else {
            Self::notify_err(window, cx, "Open a bucket first.");
            return;
        };
        let mut jobs: Vec<(PathBuf, String)> = Vec::new();
        for path in paths {
            let Some(base) = path.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
            if path.is_dir() {
                for file in walk(&path) {
                    let rel = file.strip_prefix(&path).unwrap_or(&file).to_string_lossy().replace('\\', "/");
                    jobs.push((file.clone(), format!("{prefix}{base}/{rel}")));
                }
            } else {
                jobs.push((path.clone(), format!("{prefix}{base}")));
            }
        }
        if jobs.is_empty() {
            Self::notify_err(window, cx, "Nothing to upload (empty folder?)");
            return;
        }
        let origin = s3c.origin();
        for (path, key) in jobs {
            let name = key.rsplit('/').next().unwrap_or(&key).to_string();
            let t = self.new_transfer(Direction::Upload, name, format!("{bucket}/{key}"), &path);
            let (s3c, bucket, tt, origin) = (s3c.clone(), bucket.clone(), t.clone(), origin.clone());
            self.start_transfer(
                t,
                async move {
                    let etag = s3c.upload_file(&bucket, &key, &path, tt.clone()).await?;
                    // Seed the cache with what we just uploaded: its thumbnail
                    // and later downloads then cost no egress at all.
                    let size = tt.total.load(Ordering::Relaxed);
                    if size <= SEED_ON_UPLOAD_MAX {
                        let obj = cache::ObjectRef { origin, bucket, key: key.clone(), etag, size };
                        let _ = tokio::task::spawn_blocking(move || {
                            let c = cache::global();
                            if c.store_copy(&obj, &path).is_ok() && cache::is_previewable(&key) {
                                let _ = c.thumb_from_file(&obj, &path);
                            }
                        })
                        .await;
                    }
                    Ok(())
                },
                window,
                cx,
            );
        }
    }

    pub fn download_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.selected_entries(cx);
        if entries.is_empty() {
            return;
        }
        let downloads = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
        if let [entry] = entries.as_slice() {
            if !entry.is_dir {
                let answer = cx.prompt_for_new_path(&downloads, Some(&entry.name));
                let entry = entry.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Ok(Some(dest))) = answer.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| this.download_entries(vec![entry], dest, true, window, cx));
                })
                .detach();
                return;
            }
        }
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Download here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut dirs))) = answer.await else { return };
            let Some(dir) = dirs.pop() else { return };
            let _ = this.update_in(cx, |this, window, cx| this.download_entries(entries, dir, false, window, cx));
        })
        .detach();
    }

    fn download_entries(&mut self, entries: Vec<Entry>, dest: PathBuf, exact: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((s3c, bucket, prefix)) = self.location() else { return };
        let mut folders = Vec::new();
        for entry in entries {
            if entry.is_dir {
                folders.push(entry);
                continue;
            }
            let target = if exact { dest.clone() } else { dest.join(&entry.name) };
            self.queue_download(&s3c, &bucket, entry, target, window, cx);
        }
        for folder in folders {
            let (s3c, bucket, dest, prefix) = (s3c.clone(), bucket.clone(), dest.clone(), prefix.clone());
            cx.spawn_in(window, async move |this, cx| {
                let (s, b, k) = (s3c.clone(), bucket.clone(), folder.key.clone());
                let result = s3::spawn(async move { s.list_all(&b, &k).await }).await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(objects) => {
                        let files: Vec<_> = objects.into_iter().filter(|(k, ..)| !k.ends_with('/')).collect();
                        if files.is_empty() {
                            Self::notify_err(window, cx, format!("“{}” is empty", folder.name));
                        }
                        for (key, size, etag) in files {
                            let rel = key.strip_prefix(&prefix).unwrap_or(&key);
                            let target = rel.split('/').fold(dest.clone(), |p, part| p.join(part));
                            let name = key.rsplit('/').next().unwrap_or(&key).to_string();
                            let entry = Entry { name, key: key.clone(), is_dir: false, size, modified: None, storage_class: None, etag };
                            this.queue_download(&s3c, &bucket, entry, target, window, cx);
                        }
                    }
                    Err(e) => Self::notify_err(window, cx, format!("Couldn't list “{}”: {e}", folder.name)),
                });
            })
            .detach();
        }
    }

    fn queue_download(&mut self, s3c: &S3, bucket: &str, entry: Entry, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.new_transfer(Direction::Download, entry.name.clone(), format!("{bucket}/{}", entry.key), &target);
        let (s3c, bucket, tt) = (s3c.clone(), bucket.to_string(), t.clone());
        self.start_transfer(t, async move { s3c.download_cached(&bucket, &entry, &target, tt).await.map(|_| ()) }, window, cx);
    }

    pub fn clear_finished_transfers(&mut self, cx: &mut Context<Self>) {
        self.transfers.retain(|t| t.is_active());
        cx.notify();
    }

    pub fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = !theme::palette(cx).dark;
        self.settings.dark = Some(dark);
        theme::apply(Some(dark), Some(window), cx);
        self.save_settings(cx);
    }

    pub fn set_view(&mut self, view: ViewMode, cx: &mut Context<Self>) {
        self.settings.view = view;
        self.save_settings(cx);
    }

    pub fn zoom(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.settings.zoom = (self.settings.zoom as i8 + delta).clamp(0, 2) as u8;
        self.save_settings(cx);
    }

    pub fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort = key;
            self.sort_asc = key == SortKey::Name;
        }
        cx.notify();
    }
}

/// "img2" before "img10".
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let ord = na.trim_start_matches('0').len().cmp(&nb.trim_start_matches('0').len()).then(na.cmp(&nb));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// All files under `dir`, skipping `.DS_Store`.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".DS_Store") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;

    #[test]
    fn natural_order() {
        let mut v = vec!["img10.png", "img2.png", "IMG1.png", "a.txt"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.txt", "IMG1.png", "img2.png", "img10.png"]);
    }
}
