//! Drag & drop payloads and the shared drop rule. Targets call `accepts` (lexical, runs every
//! frame) to decide whether to light up and `perform` to run the drop through `FileOps`.

use crate::fm_core::dnd::{DragSource, drop_op, request, same_volume};
use crate::fm_core::layout::PaneId;
use crate::fm_core::places::Section;
use crate::ui::ops::FileOps;
use crate::ui::theme;
use gpui::{App, Context, Entity, ExternalPaths, IntoElement, Render, Window, div, prelude::*, px, rgb};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Files dragged out of a list row. `parents` and `all_dirs` are computed once at drag start.
#[derive(Clone)]
pub struct DraggedFiles {
    pub paths: Arc<[PathBuf]>,
    pub parents: Arc<[PathBuf]>,
    /// Every dragged entry is a folder (what the sidebar headings accept), from the listing.
    pub all_dirs: bool,
}

impl DraggedFiles {
    pub fn new(paths: Vec<PathBuf>, all_dirs: bool) -> Self {
        let parents = paths.iter().map(|p| p.parent().map(Path::to_path_buf).unwrap_or_default()).collect::<Vec<_>>();
        Self { paths: paths.into(), parents: parents.into(), all_dirs }
    }
}

/// A tab dragged within its pane's tab strip.
#[derive(Clone, Copy)]
pub struct DraggedTab {
    pub pane: PaneId,
    pub ix: usize,
}

/// A sidebar entry dragged to reorder.
#[derive(Clone, Copy)]
pub struct DraggedPlace {
    pub group: Section,
    pub ix: usize,
}

/// The ghost under the cursor: a plain div so it never takes the hover hitbox from drop targets.
pub struct DragPreview {
    label: String,
}

impl DragPreview {
    pub fn for_paths(paths: &[PathBuf]) -> Self {
        let label = match paths {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            many => format!("{} items", many.len()),
        };
        Self { label }
    }

    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into() }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(theme::SELECTED_BG))
            .text_sm()
            .text_color(rgb(theme::TEXT))
            .max_w(px(260.))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

/// Whether dropping `payload` onto folder `dest` would do something. Lexical and cheap: it runs
/// at paint time for every drop target. False while a file operation runs, so a drop never
/// silently vanishes.
pub fn accepts(payload: &dyn Any, dest: &Path, alt: bool, ops_busy: bool) -> bool {
    if ops_busy {
        return false;
    }
    if let Some(d) = payload.downcast_ref::<DraggedFiles>() {
        return drop_op(&d.paths, &d.parents, dest, alt, DragSource::Internal, &|_, _| true).is_some();
    }
    if let Some(e) = payload.downcast_ref::<ExternalPaths>() {
        let parents: Vec<PathBuf> = Vec::new();
        return drop_op(e.paths(), &parents, dest, false, DragSource::External, &|_, _| true).is_some();
    }
    false
}

/// Runs a drop through `FileOps` (undoable, no selection change). Returns whether anything started.
pub fn perform(ops: &Entity<FileOps>, srcs: &[PathBuf], dest: &Path, alt: bool, source: DragSource, cx: &mut App) -> bool {
    if ops.read(cx).busy() || !dest.is_dir() {
        return false;
    }
    let Ok(canon_dest) = std::fs::canonicalize(dest) else { return false };
    // Canonicalize the parent folders, not the entries: a dragged symlink must stay a symlink.
    let canon: Vec<PathBuf> = srcs
        .iter()
        .filter_map(|s| {
            let name = s.file_name()?;
            Some(std::fs::canonicalize(s.parent()?).ok()?.join(name))
        })
        .collect();
    let parents: Vec<PathBuf> = canon.iter().filter_map(|s| s.parent().map(Path::to_path_buf)).collect();
    let Some(op) = drop_op(&canon, &parents, &canon_dest, alt, source, &same_volume) else { return false };
    // The rule is decided on canonical paths; the operation keeps the spelling the lists use so
    // the lists whose folders it touches recognise them and reload.
    let req = request(op, srcs.to_vec(), dest.to_path_buf());
    ops.update(cx, |o, cx| o.run(req, None, false, cx));
    true
}
