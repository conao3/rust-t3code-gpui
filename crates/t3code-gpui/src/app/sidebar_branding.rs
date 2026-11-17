use super::*;
use crate::provider_display::{Entry, accent_foreground, initials};

fn hex(value: &str) -> Hsla {
    rgb(u32::from_str_radix(&value[1..], 16).unwrap()).into()
}

fn provider_icon(entry: &Entry, previous: bool, light: bool, cx: &App) -> AnyElement {
    let size = if previous { 12. } else { 14. };
    let theme = if light { "light" } else { "dark" };
    let driver = if entry.driver == "openai" {
        "codex"
    } else {
        &entry.driver
    };
    let path = SharedString::from(format!("icons/providers/{driver}-{theme}.svg"));
    let source = crate::appearance::Assets
        .load(&path)
        .ok()
        .flatten()
        .map(|_| path.clone());
    let mut element = div().relative().flex_none().size(px(size));
    let icon = if matches!(
        driver,
        "codex" | "anthropic" | "openrouter" | "ollama" | "acpRegistry"
    ) {
        Icon::default()
            .path(path.clone())
            .size(px(size))
            .when(driver == "acpRegistry", |icon| {
                icon.text_color(cx.theme().muted_foreground)
            })
            .into_any_element()
    } else if let Some(source) = source {
        img(source)
            .size(px(size))
            .aspect_ratio(1.)
            .grayscale(previous)
            .into_any_element()
    } else {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(if entry.driver == "openaiCompatible" {
                9.
            } else {
                10.
            }))
            .line_height(px(size))
            .font_weight(FontWeight::SEMIBOLD)
            .when(entry.driver == "openaiCompatible", |tile| {
                tile.rounded(px(6.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
            })
            .child(if entry.driver == "openaiCompatible" {
                initials(&entry.name).chars().take(1).collect()
            } else {
                initials(&entry.name)
            })
            .into_any_element()
    };
    element = element.child(
        div()
            .size_full()
            .opacity(if previous { 0.35 } else { 0.6 })
            .child(icon),
    );
    if !previous && entry.badge && entry.driver != "openaiCompatible" {
        let (background, foreground) = entry
            .accent
            .as_deref()
            .map(|accent| (hex(accent), hex(accent_foreground(accent))))
            .unwrap_or((cx.theme().popover, cx.theme().muted_foreground));
        element = element.child(
            div()
                .absolute()
                .right(px(-3.))
                .bottom(px(-3.))
                .h(px(12.))
                .min_w(px(12.))
                .px(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(cx.theme().popover)
                .bg(background)
                .text_color(foreground)
                .text_size(px(8.))
                .line_height(px(8.))
                .font_weight(FontWeight::SEMIBOLD)
                .shadow_sm()
                .child(initials(&entry.name)),
        );
    }
    element.into_any_element()
}

impl Root {
    pub(super) fn host_label(&self, environment: &str) -> String {
        let config = self.config_of(environment);
        let label = config
            .as_ref()
            .and_then(|config| config.pointer("/environment/label"))
            .and_then(Value::as_str)
            .unwrap_or(environment);
        match label {
            "conao-nixos-eos" => "eos",
            "conao-nixos-helios" => "helios",
            label => label,
        }
        .to_string()
    }

    pub(super) fn render_host_chip(
        &self,
        environment: &str,
        remote: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let machine = self
            .sidebar_catalogs
            .get(environment)
            .map(|catalog| catalog.machine.as_str())
            .unwrap_or("server");
        let icon = match machine {
            "desktop" => gpui_kit::assets::IconName::Monitor,
            "laptop" => gpui_kit::assets::IconName::Laptop,
            "cloud" => gpui_kit::assets::IconName::Cloud,
            _ => gpui_kit::assets::IconName::Server,
        };
        let machine_icon = if matches!(machine, "linux" | "mac-mini" | "mac-studio") {
            Icon::default()
                .path(SharedString::from(if machine == "linux" {
                    "icons/providers/linux-dark.svg".to_string()
                } else {
                    format!("icons/providers/{machine}.svg")
                }))
                .size(px(12.))
                .into_any_element()
        } else {
            Icon::new(icon).size(px(12.)).into_any_element()
        };
        h_flex()
            .h(px(20.))
            .min_w_0()
            .flex_shrink(1.)
            .px(px(6.))
            .gap_1()
            .rounded_sm()
            .text_size(px(12.))
            .line_height(px(20.))
            .font_weight(FontWeight::MEDIUM)
            .bg(if remote {
                if self.settings.light_theme {
                    rgba(0xf0b10033).into()
                } else {
                    rgba(0xffb90033).into()
                }
            } else {
                transparent_black()
            })
            .text_color(if remote {
                rgb(if self.settings.light_theme {
                    0x973c00
                } else {
                    0xfee685
                })
                .into()
            } else {
                crate::appearance::secondary_label(cx)
            })
            .child(machine_icon)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(settings_controls::text(self.host_label(environment))),
            )
            .into_any_element()
    }

    pub(super) fn render_sidebar_provider_stack(
        &self,
        entry: &ThreadEntry,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let catalog = self.sidebar_catalogs.get(&entry.environment)?;
        let current = entry.provider_stack.last()?;
        catalog
            .entries
            .iter()
            .find(|entry| &entry.instance_id == current)?;
        Some(
            h_flex()
                .flex_none()
                .gap(px(6.))
                .children(entry.provider_stack.iter().enumerate().filter_map(
                    |(index, instance)| {
                        catalog
                            .entries
                            .iter()
                            .find(|provider| &provider.instance_id == instance)
                            .map(|provider| {
                                provider_icon(
                                    provider,
                                    index + 1 < entry.provider_stack.len(),
                                    self.settings.light_theme,
                                    cx,
                                )
                            })
                    },
                ))
                .into_any_element(),
        )
    }
}
