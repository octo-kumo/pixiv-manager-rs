//! Twitter viewer transforms: recombine split images and rotate pages.
//!
//! Everything here is display-only. The chosen transform per tweet lives in
//! `AppState::view_transforms` (persisted), the rendered result in
//! `ViewerState::transformed`. [`sync`] runs after every update and starts
//! whatever render or split probe the current viewer state calls for.

use super::fetch_pfp;
use super::viewer::full_image_url;
use crate::app::message::Message;
use crate::app::state::AppState;
use crate::media::Provider;
use crate::viewer_tools::{self, TransformKey, TransformTarget, ViewTransform, MERGED_SLOT};

/// Whether the transform tools apply to the currently open viewer item.
pub fn enabled(state: &AppState) -> bool {
    state.viewer.open
        && state.settings.twitter_viewer_tools
        && crate::media::provider_of(state.viewer.illust_id) == Provider::Twitter
}

/// The transform the viewer should currently display, if any.
pub fn desired_key(state: &AppState) -> Option<TransformKey> {
    if !enabled(state) {
        return None;
    }
    let id = state.viewer.illust_id;
    let tx = state.view_transforms.get(&id)?;
    match tx.merge {
        Some(axis) if state.viewer.total_pages >= 2 => Some(TransformKey {
            id,
            target: TransformTarget::Merged(axis),
            rotation: tx.rotation(MERGED_SLOT),
        }),
        _ => {
            let page = state.viewer.page;
            let rotation = tx.rotation(page);
            (rotation != 0).then_some(TransformKey {
                id,
                target: TransformTarget::Page(page),
                rotation,
            })
        }
    }
}

/// The handle the viewer should draw: the rendered transform when it matches
/// what is wanted, else the untransformed page.
pub fn display_handle(state: &AppState) -> Option<&iced::widget::image::Handle> {
    if let (Some((key, handle)), Some(want)) = (&state.viewer.transformed, desired_key(state)) {
        if *key == want {
            return Some(handle);
        }
    }
    state.viewer.handle.as_ref()
}

fn edit_transform(state: &mut AppState, f: impl FnOnce(&mut ViewTransform, usize)) {
    let id = state.viewer.illust_id;
    let page = state.viewer.page;
    let tx = state.view_transforms.entry(id).or_default();
    f(tx, page);
    if tx.is_identity() {
        state.view_transforms.remove(&id);
    }
    state.viewer.transform_failed = None;
    viewer_tools::save(&state.view_transforms);
}

/// Handle the transform messages.
pub(super) fn handle(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::ViewerMergeToggled(axis) if enabled(state) => {
            edit_transform(state, |tx, _| tx.toggle_merge(axis));
        }
        Message::ViewerRotate(quarters) if enabled(state) => {
            edit_transform(state, |tx, page| {
                let slot = if tx.merge.is_some() {
                    MERGED_SLOT
                } else {
                    page
                };
                tx.rotate(slot, quarters);
            });
        }
        Message::ViewerTransformReset if enabled(state) => {
            edit_transform(state, |tx, _| *tx = ViewTransform::default());
        }
        Message::ViewerSplitDetected(id, axis) => {
            if state.viewer.open && state.viewer.illust_id == id {
                state.viewer.split_hint = axis;
            }
        }
        Message::ViewerTransformReady(key, result) => {
            if state.viewer.transform_pending == Some(key) {
                state.viewer.transform_pending = None;
            }
            match result {
                Ok(handle) => state.viewer.transformed = Some((key, handle)),
                Err(e) => {
                    log::warn!("[viewer_tools] transform failed: {e}");
                    state.viewer.transform_failed = Some(key);
                }
            }
        }
        _ => {}
    }
    iced::Task::none()
}

/// Start the render / probe the current viewer state needs, if any. Cheap
/// when nothing is needed, so it runs after every update.
pub fn sync(state: &mut AppState) -> iced::Task<Message> {
    if !enabled(state) {
        return iced::Task::none();
    }
    let mut tasks = Vec::new();
    if let Some(task) = start_render(state) {
        tasks.push(task);
    }
    if let Some(task) = start_probe(state) {
        tasks.push(task);
    }
    iced::Task::batch(tasks)
}

fn start_render(state: &mut AppState) -> Option<iced::Task<Message>> {
    let want = desired_key(state)?;
    let already = state.viewer.transformed.as_ref().map(|(k, _)| *k) == Some(want);
    if already
        || state.viewer.transform_pending == Some(want)
        || state.viewer.transform_failed == Some(want)
    {
        return None;
    }
    match want.target {
        TransformTarget::Page(page) => {
            // Rotation works on the page's already-loaded bytes; wait until
            // the page lands (sync re-runs after FullImageLoaded).
            let iced::widget::image::Handle::Bytes(_, bytes) =
                state.viewer.page_handles.get(&page)?
            else {
                return None;
            };
            let bytes = bytes.clone();
            state.viewer.transform_pending = Some(want);
            Some(iced::Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let img = viewer_tools::decode(&bytes)?;
                        Ok(rgba_handle(viewer_tools::rotate(img, want.rotation)))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                move |r| Message::ViewerTransformReady(want, r),
            ))
        }
        TransformTarget::Merged(axis) => {
            let illust = state.viewer.illust.as_ref()?;
            let sources: Vec<PageSource> = (0..state.viewer.total_pages)
                .map(|p| PageSource::new(state, p, full_image_url(illust, p)))
                .collect::<Option<_>>()?;
            state.viewer.transform_pending = Some(want);
            Some(iced::Task::perform(
                async move {
                    let mut pages = Vec::with_capacity(sources.len());
                    for source in sources {
                        let bytes = source.bytes().await?;
                        let decoded =
                            tokio::task::spawn_blocking(move || viewer_tools::decode(&bytes))
                                .await
                                .map_err(|e| e.to_string())??;
                        pages.push(decoded);
                    }
                    tokio::task::spawn_blocking(move || {
                        let combined = viewer_tools::combine(&pages, axis);
                        Ok(rgba_handle(viewer_tools::rotate(combined, want.rotation)))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                move |r| Message::ViewerTransformReady(want, r),
            ))
        }
    }
}

/// Look for a split image once per opened tweet, after its first page has
/// loaded (so the probe never competes with the page being looked at).
fn start_probe(state: &mut AppState) -> Option<iced::Task<Message>> {
    let id = state.viewer.illust_id;
    let pages = state.viewer.total_pages;
    if state.viewer.split_probe == Some(id)
        || !(2..=8).contains(&pages)
        || state
            .view_transforms
            .get(&id)
            .is_some_and(|t| t.merge.is_some())
        || !state.viewer.page_handles.contains_key(&state.viewer.page)
    {
        return None;
    }
    let illust = state.viewer.illust.as_ref()?;
    // The 1200px variants are plenty to judge seams (and thumbnails already
    // cache the first one).
    let sources: Vec<PageSource> = (0..pages)
        .map(|p| {
            let url = illust
                .meta_pages
                .get(p)
                .and_then(|m| m.image_urls.large.clone().or(m.image_urls.original.clone()))?;
            PageSource::new(state, p, Some(url))
        })
        .collect::<Option<_>>()?;
    state.viewer.split_probe = Some(id);
    Some(iced::Task::perform(
        async move {
            let mut pages = Vec::with_capacity(sources.len());
            for source in sources {
                let Ok(bytes) = source.bytes().await else {
                    return None;
                };
                let decoded =
                    tokio::task::spawn_blocking(move || viewer_tools::decode(&bytes).ok())
                        .await
                        .ok()??;
                pages.push(decoded);
            }
            tokio::task::spawn_blocking(move || viewer_tools::detect_split(&pages))
                .await
                .ok()?
        },
        move |axis| Message::ViewerSplitDetected(id, axis),
    ))
}

fn rgba_handle(img: image::RgbaImage) -> iced::widget::image::Handle {
    let (w, h) = (img.width(), img.height());
    iced::widget::image::Handle::from_rgba(w, h, img.into_raw())
}

/// Where to get one page's encoded bytes: the downloaded original on disk if
/// present, else the (cached) network image.
struct PageSource {
    url: String,
    disk: Option<std::path::PathBuf>,
    cache: std::sync::Arc<crate::image::cache::ImageCache>,
    fetcher: crate::app::state::ImageFetcher,
}

impl PageSource {
    fn new(state: &AppState, page: usize, url: Option<String>) -> Option<Self> {
        let url = url?;
        let gallery = state
            .settings
            .gallery_path
            .clone()
            .unwrap_or_else(crate::core::default_gallery_dir);
        let id = state.viewer.illust_id;
        let disk = crate::core::migrate_download_dir(&gallery, id)
            .and_then(|dir| crate::core::find_page_file(&dir, id, page));
        Some(Self {
            url,
            disk,
            cache: state.images.image_cache.clone(),
            fetcher: state.images.fetcher.clone(),
        })
    }

    async fn bytes(self) -> Result<Vec<u8>, String> {
        if let Some(path) = &self.disk {
            let path = path.clone();
            if let Ok(Ok(bytes)) = tokio::task::spawn_blocking(move || std::fs::read(path)).await {
                return Ok(bytes);
            }
        }
        fetch_pfp(self.url, self.cache, self.fetcher, Provider::Twitter)
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer_tools::MergeAxis;

    fn twitter_viewer() -> AppState {
        let mut state = AppState::default();
        state.viewer.open = true;
        state.viewer.illust_id = crate::media::twitter_key(42);
        state.viewer.total_pages = 3;
        state
    }

    #[test]
    fn rotate_and_undo_roundtrip() {
        let mut state = twitter_viewer();
        let id = state.viewer.illust_id;
        handle(&mut state, Message::ViewerRotate(1));
        assert_eq!(
            desired_key(&state),
            Some(TransformKey {
                id,
                target: TransformTarget::Page(0),
                rotation: 1
            })
        );
        handle(&mut state, Message::ViewerRotate(-1));
        assert!(desired_key(&state).is_none());
        assert!(!state.view_transforms.contains_key(&id));
    }

    #[test]
    fn merge_rotation_is_separate_from_page_rotation() {
        let mut state = twitter_viewer();
        handle(&mut state, Message::ViewerRotate(1));
        handle(
            &mut state,
            Message::ViewerMergeToggled(MergeAxis::Horizontal),
        );
        let key = desired_key(&state).unwrap();
        assert_eq!(key.target, TransformTarget::Merged(MergeAxis::Horizontal));
        assert_eq!(key.rotation, 0);
        handle(&mut state, Message::ViewerRotate(2));
        assert_eq!(desired_key(&state).unwrap().rotation, 2);
        // Clicking the same axis again undoes the merge; the page's own
        // rotation is still there.
        handle(
            &mut state,
            Message::ViewerMergeToggled(MergeAxis::Horizontal),
        );
        assert_eq!(
            desired_key(&state).unwrap().target,
            TransformTarget::Page(0)
        );
        handle(&mut state, Message::ViewerTransformReset);
        assert!(desired_key(&state).is_none());
    }

    #[test]
    fn tools_ignore_non_twitter_and_disabled() {
        let mut state = twitter_viewer();
        state.settings.twitter_viewer_tools = false;
        handle(&mut state, Message::ViewerRotate(1));
        assert!(state.view_transforms.is_empty());

        let mut pixiv = AppState::default();
        pixiv.viewer.open = true;
        pixiv.viewer.illust_id = 7;
        handle(&mut pixiv, Message::ViewerRotate(1));
        assert!(pixiv.view_transforms.is_empty());
    }
}
