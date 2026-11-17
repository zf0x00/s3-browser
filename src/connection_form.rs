//! The "New / Edit connection" form shown in a dialog.

use anyhow::{Result, bail};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Entity, FontWeight, SharedString, Window, div, px, rgb};

use crate::providers::{PROVIDERS, ProviderKind};
use crate::s3::{self, S3};
use crate::store::{self, Connection, ImportedCredential, Secret};

pub enum FormStatus {
    Idle,
    Testing,
    Ok(String),
    Err(String),
}

pub struct ConnectionForm {
    /// `Some(id)` when editing a saved connection.
    pub editing: Option<String>,
    pub provider: ProviderKind,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    region: Entity<InputState>,
    access_key: Entity<InputState>,
    secret_key: Entity<InputState>,
    session_token: Entity<InputState>,
    bucket: Entity<InputState>,
    pub path_style: bool,
    last_region: String,
    has_saved_secret: bool,
    pub status: FormStatus,
}

impl ConnectionForm {
    pub fn new(existing: Option<Connection>, provider: Option<ProviderKind>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kind = existing.as_ref().map(|c| c.provider).or(provider).unwrap_or(ProviderKind::Aws);
        let info = kind.info();
        let (name, endpoint, region, access, bucket, path_style) = match &existing {
            Some(c) => (c.name.clone(), c.endpoint.clone(), c.region.clone(), c.access_key.clone(), c.bucket.clone().unwrap_or_default(), c.path_style),
            None => (
                info.name.to_string(),
                info.endpoint_for(info.default_region),
                info.default_region.to_string(),
                String::new(),
                String::new(),
                info.path_style,
            ),
        };
        let input = |window: &mut Window, cx: &mut Context<Self>, placeholder: &str, value: String| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let name_in = input(window, cx, "My storage", name);
        let endpoint_in = input(window, cx, "https://s3.example.com  (empty = AWS default)", endpoint);
        let region_in = input(window, cx, "us-east-1", region.clone());
        let access_in = input(window, cx, "AKIA…", access);
        let editing = existing.as_ref().map(|c| c.id.clone());
        let secret_placeholder = if editing.is_some() { "Leave empty to keep the saved key" } else { "Secret access key" };
        let secret_in = cx.new(|cx| InputState::new(window, cx).placeholder(secret_placeholder).masked(true));
        let token_in = cx.new(|cx| InputState::new(window, cx).placeholder("Only for temporary (STS) credentials").masked(true));
        let bucket_in = input(window, cx, "Needed if the key can't list buckets", bucket);

        // Keep a templated endpoint in step with the region (e.g. B2, Wasabi).
        cx.subscribe_in(&region_in, window, |this: &mut Self, state, event: &InputEvent, window, cx| {
            if let InputEvent::Change = event {
                let new_region = state.read(cx).value().to_string();
                let info = this.provider.info();
                let current = this.endpoint.read(cx).value().to_string();
                if info.endpoint.contains("{region}") && current == info.endpoint_for(&this.last_region) {
                    let next = info.endpoint_for(&new_region);
                    this.endpoint.update(cx, |s, cx| s.set_value(next, window, cx));
                }
                this.last_region = new_region;
            }
        })
        .detach();

        Self {
            editing,
            provider: kind,
            name: name_in,
            endpoint: endpoint_in,
            region: region_in,
            access_key: access_in,
            secret_key: secret_in,
            session_token: token_in,
            bucket: bucket_in,
            path_style,
            last_region: region,
            has_saved_secret: existing.is_some(),
            status: FormStatus::Idle,
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.editing.is_some() { &self.name } else { &self.access_key };
        target.update(cx, |s, cx| s.focus(window, cx));
    }

    fn set_value(state: &Entity<InputState>, value: String, window: &mut Window, cx: &mut Context<Self>) {
        state.update(cx, |s, cx| s.set_value(value, window, cx));
    }

    pub fn set_provider(&mut self, kind: ProviderKind, window: &mut Window, cx: &mut Context<Self>) {
        if kind == self.provider {
            return;
        }
        let old = self.provider.info();
        let new = kind.info();
        // Replace fields that still hold the previous provider's defaults.
        let name = self.name.read(cx).value().to_string();
        if name.is_empty() || PROVIDERS.iter().any(|p| p.name == name) {
            Self::set_value(&self.name, new.name.to_string(), window, cx);
        }
        let region = self.region.read(cx).value().to_string();
        let region = if region.is_empty() || old.regions.contains(&region.as_str()) {
            Self::set_value(&self.region, new.default_region.to_string(), window, cx);
            new.default_region.to_string()
        } else {
            region
        };
        let endpoint = self.endpoint.read(cx).value().to_string();
        if endpoint.is_empty() || endpoint == old.endpoint_for(&self.last_region) || endpoint == old.endpoint {
            Self::set_value(&self.endpoint, new.endpoint_for(&region), window, cx);
        }
        self.last_region = region;
        self.path_style = new.path_style;
        self.provider = kind;
        self.status = FormStatus::Idle;
        cx.notify();
    }

    pub fn apply_import(&mut self, cred: &ImportedCredential, window: &mut Window, cx: &mut Context<Self>) {
        Self::set_value(&self.access_key, cred.access_key.clone(), window, cx);
        Self::set_value(&self.secret_key, cred.secret_key.clone(), window, cx);
        Self::set_value(&self.session_token, cred.session_token.clone().unwrap_or_default(), window, cx);
        if let Some(region) = &cred.region {
            Self::set_value(&self.region, region.clone(), window, cx);
        }
        if let Some(endpoint) = &cred.endpoint {
            Self::set_value(&self.endpoint, endpoint.clone(), window, cx);
        }
        if let Some(bucket) = &cred.bucket {
            Self::set_value(&self.bucket, bucket.clone(), window, cx);
        }
        self.status = FormStatus::Ok(format!("Imported {}", cred.label));
        cx.notify();
    }

    /// Validate and assemble the connection. The secret is `None` when editing
    /// and the field was left empty (keep the saved one).
    pub fn build(&self, cx: &gpui_kit::App) -> Result<(Connection, Option<Secret>)> {
        let v = |s: &Entity<InputState>| s.read(cx).value().trim().to_string();
        let name = v(&self.name);
        let endpoint = v(&self.endpoint);
        let access_key = v(&self.access_key);
        let secret_key = v(&self.secret_key);
        if access_key.is_empty() {
            bail!("Enter an access key ID.");
        }
        if secret_key.is_empty() && !self.has_saved_secret {
            bail!("Enter the secret access key.");
        }
        if endpoint.contains('{') {
            bail!("Fill in the placeholder in the endpoint URL (e.g. your account ID).");
        }
        if !endpoint.is_empty() && !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            bail!("The endpoint must start with https:// or http://");
        }
        if endpoint == "https://" || endpoint == "http://" {
            bail!("Enter the endpoint URL of your S3-compatible service.");
        }
        if self.provider != ProviderKind::Aws && endpoint.is_empty() {
            bail!("This provider needs an endpoint URL.");
        }
        let bucket = v(&self.bucket);
        let conn = Connection {
            id: self.editing.clone().unwrap_or_else(store::new_id),
            name: if name.is_empty() { self.provider.info().name.to_string() } else { name },
            provider: self.provider,
            endpoint,
            region: v(&self.region),
            access_key,
            path_style: self.path_style,
            bucket: if bucket.is_empty() { None } else { Some(bucket) },
        };
        let token = v(&self.session_token);
        let secret = if secret_key.is_empty() {
            None
        } else {
            Some(Secret { secret_key, session_token: if token.is_empty() { None } else { Some(token) } })
        };
        Ok((conn, secret))
    }

    /// Try ListBuckets with the current values.
    pub fn test(&mut self, cx: &mut Context<Self>) {
        let (conn, secret) = match self.build(cx) {
            Ok(v) => v,
            Err(e) => {
                self.status = FormStatus::Err(e.to_string());
                cx.notify();
                return;
            }
        };
        self.status = FormStatus::Testing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = s3::spawn(async move {
                let secret = match secret {
                    Some(s) => s,
                    None => {
                        let id = conn.id.clone();
                        tokio::task::spawn_blocking(move || store::load_secret(&id)).await??
                    }
                };
                S3::new(&conn, &secret).list_buckets().await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(b) if b.is_empty() => FormStatus::Ok("Connected — no buckets yet.".into()),
                    Ok(b) => FormStatus::Ok(format!(
                        "Connected — {} bucket{} found.",
                        b.len(),
                        if b.len() == 1 { "" } else { "s" }
                    )),
                    Err(e) => FormStatus::Err(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn field(label: &'static str, input: &Entity<InputState>, masked: bool) -> AnyElement {
        let mut field = Input::new(input);
        if masked {
            field = field.mask_toggle();
        }
        v_flex()
            .flex_1()
            .gap(px(5.))
            .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(label))
            .child(field)
            .into_any_element()
    }
}

/// The provider's logo on its brand colour, like an app icon.
pub fn provider_badge(kind: ProviderKind, size: f32) -> impl IntoElement {
    let info = kind.info();
    let glyph = px(size * 0.6);
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(size * 0.26))
        .bg(rgb(info.color))
        .flex()
        .items_center()
        .justify_center()
        .child(if info.logo.is_empty() {
            Icon::new(IconName::Server).size(glyph).text_color(rgb(info.glyph)).into_any_element()
        } else {
            gpui_kit::svg().path(info.logo).size(glyph).text_color(rgb(info.glyph)).into_any_element()
        })
}

impl Render for ConnectionForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = cx.theme().colors;
        let info = self.provider.info();

        let chips = PROVIDERS.iter().map(|p| {
            let kind = p.kind;
            let selected = kind == self.provider;
            h_flex()
                .id(SharedString::from(format!("provider-{}", p.short)))
                .gap(px(7.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(if selected { c.primary } else { c.border })
                .when(selected, |d| d.bg(c.accent))
                .cursor_pointer()
                .hover(|s| s.bg(c.accent))
                .child(provider_badge(kind, 20.))
                .child(div().text_xs().child(p.name))
                .on_click(cx.listener(move |this, _, window, cx| this.set_provider(kind, window, cx)))
        });

        let regions = info.regions.iter().take(10).map(|r| {
            let region = r.to_string();
            div()
                .id(SharedString::from(format!("region-{r}")))
                .px(px(6.))
                .py(px(2.))
                .rounded(px(5.))
                .bg(c.muted)
                .text_xs()
                .text_color(c.muted_foreground)
                .cursor_pointer()
                .hover(|s| s.text_color(c.foreground))
                .child(region.clone())
                .on_click(cx.listener(move |this, _, window, cx| {
                    let region_state = this.region.clone();
                    region_state.update(cx, |s, cx| s.set_value(region.clone(), window, cx));
                }))
        });

        let status = match &self.status {
            FormStatus::Idle => None,
            FormStatus::Testing => Some((c.muted_foreground, IconName::LoaderCircle, "Testing connection…".to_string())),
            FormStatus::Ok(m) => Some((c.success, IconName::CircleCheck, m.clone())),
            FormStatus::Err(m) => Some((c.danger, IconName::CircleAlert, m.clone())),
        };

        v_flex()
            .gap(px(14.))
            .text_color(c.foreground)
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).child("Provider"))
                    .child(h_flex().flex_wrap().gap(px(6.)).children(chips))
                    .child(div().text_xs().text_color(c.muted_foreground).child(info.hint)),
            )
            .child(Self::field("Display name", &self.name, false))
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_start()
                    .child(div().flex_1().child(Self::field("Endpoint URL", &self.endpoint, false)))
                    .child(div().w(px(170.)).child(Self::field("Region", &self.region, false))),
            )
            .child(h_flex().flex_wrap().gap(px(4.)).children(regions))
            .child(
                h_flex()
                    .gap(px(10.))
                    .child(Self::field("Access key ID", &self.access_key, false))
                    .child(Self::field("Secret access key", &self.secret_key, true)),
            )
            .child(
                h_flex()
                    .gap(px(10.))
                    .child(Self::field("Session token (optional)", &self.session_token, true))
                    .child(Self::field("Default bucket (optional)", &self.bucket, false)),
            )
            .child(
                h_flex()
                    .gap(px(10.))
                    .child(
                        Switch::new("path-style")
                            .checked(self.path_style)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.path_style = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .child(div().text_sm().child("Path-style requests"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.muted_foreground)
                                    .child("endpoint/bucket/key instead of bucket.endpoint/key (s3fs: use_path_request_style)"),
                            ),
                    ),
            )
            .when_some(status, |d, (color, icon, msg)| {
                d.child(
                    h_flex()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(c.muted)
                        .text_sm()
                        .text_color(color)
                        .child(Icon::new(icon).size(px(15.)).text_color(color))
                        .child(div().flex_1().child(msg)),
                )
            })
    }
}
