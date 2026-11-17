use super::*;
use serde_json::Value;
use t3_client::session::Connection;

#[derive(Default)]
pub(super) struct Images {
    source: Vec<Value>,
    items: Vec<(String, Arc<Image>)>,
    error: Option<String>,
}

impl RowView {
    pub fn load_images(
        &mut self,
        source: Vec<Value>,
        connection: Arc<Connection>,
        runtime: tokio::runtime::Handle,
        cx: &mut Context<Self>,
    ) {
        if self.images.source == source {
            return;
        }
        self.images.source = source.clone();
        self.images.items.clear();
        self.images.error = None;
        if source.is_empty() {
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            let expected = source.clone();
            let result = runtime
                .spawn(async move {
                    let mut images = vec![];
                    for item in source {
                        let name = item["name"].as_str().unwrap_or("Image").to_string();
                        let mime = item["mimeType"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Image MIME type missing"))?;
                        let format = ImageFormat::from_mime_type(mime)
                            .ok_or_else(|| anyhow::anyhow!("Unsupported image type: {mime}"))?;
                        let bytes = connection.attachment_bytes(item.clone()).await?;
                        images.push((name, Arc::new(Image::from_bytes(format, bytes))));
                    }
                    Ok::<_, anyhow::Error>(images)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.images.source != expected {
                    return;
                }
                match result {
                    Ok(Ok(items)) => view.images.items = items,
                    Ok(Err(error)) => {
                        view.images.error = Some(format!("Could not load image: {error:#}"))
                    }
                    Err(error) => {
                        view.images.error = Some(format!("Could not load image: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_images(&self) -> AnyElement {
        v_flex()
            .gap_2()
            .children(
                self.images
                    .items
                    .iter()
                    .enumerate()
                    .map(|(index, (name, image))| {
                        div()
                            .id(SharedString::from(format!("image-{index}")))
                            .role(accesskit::Role::Image)
                            .aria_label(name.clone())
                            .child(
                                img(image.clone())
                                    .max_w(px(400.))
                                    .max_h(px(300.))
                                    .object_fit(ObjectFit::Contain)
                                    .rounded_md(),
                            )
                    }),
            )
            .children(self.images.error.clone())
            .into_any_element()
    }
}
