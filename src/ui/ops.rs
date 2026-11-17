//! UI-side owner of the undo stack, the internal clipboard and the "one operation at a time" gate.

use crate::fm_core::ops::{OpRequest, perform, reapply, revert};
use crate::fm_core::undo::UndoStack;
use gpui::{ClipboardItem, Context, EntityId, EventEmitter, Task};
use std::path::PathBuf;

pub enum FileOpsEvent {
    /// An operation (or undo/redo) finished; `touched` directories may have changed. Only the
    /// `origin` list applies `select`/`rename`; undo/redo have no origin.
    Done { touched: Vec<PathBuf>, select: Vec<PathBuf>, rename: bool, origin: Option<EntityId> },
    Error(String),
    StackChanged,
}

pub struct FileOps {
    stack: UndoStack,
    busy: bool,
    clipboard: Vec<PathBuf>,
    task: Option<Task<()>>,
}

impl EventEmitter<FileOpsEvent> for FileOps {}

impl Default for FileOps {
    fn default() -> Self {
        Self::new()
    }
}

impl FileOps {
    pub fn new() -> Self {
        Self { stack: UndoStack::default(), busy: false, clipboard: Vec::new(), task: None }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn can_undo(&self) -> bool {
        self.stack.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.stack.can_redo()
    }

    pub fn undo_title(&self) -> Option<String> {
        self.stack.undo_title().map(str::to_string)
    }

    pub fn redo_title(&self) -> Option<String> {
        self.stack.redo_title().map(str::to_string)
    }

    pub fn clipboard(&self) -> &[PathBuf] {
        &self.clipboard
    }

    /// Remembers the paths internally (gpui 0.2.2 cannot carry file URLs) and mirrors them as text.
    pub fn set_clipboard(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let text = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        self.clipboard = paths;
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub fn run(&mut self, req: OpRequest, origin: EntityId, rename_after: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let is_move = matches!(req, OpRequest::Move { .. });
        self.busy = true;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { perform(req) }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match res {
                    Ok(out) => {
                        let changed = out.record.is_some();
                        if let Some(r) = out.record {
                            this.stack.push(r);
                        }
                        if is_move {
                            this.clipboard.clear();
                        }
                        cx.emit(FileOpsEvent::Done {
                            touched: out.touched,
                            select: out.results,
                            rename: rename_after,
                            origin: Some(origin),
                        });
                        if changed {
                            cx.emit(FileOpsEvent::StackChanged);
                        }
                    }
                    Err(e) => cx.emit(FileOpsEvent::Error(e.to_string())),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn step(&mut self, undo: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let rec = if undo { self.stack.peek_undo() } else { self.stack.peek_redo() };
        let Some(mut rec) = rec else { return };
        self.busy = true;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let (rec, res) = cx
                .background_executor()
                .spawn(async move {
                    let res = if undo { revert(&mut rec) } else { reapply(&mut rec) };
                    (rec, res)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                let touched = rec.touched_dirs();
                if undo {
                    this.stack.finish_undo(rec, &res);
                } else {
                    this.stack.finish_redo(rec, &res);
                }
                match res {
                    Ok(()) => cx.emit(FileOpsEvent::Done { touched, select: vec![], rename: false, origin: None }),
                    Err(e) => cx.emit(FileOpsEvent::Error(e.to_string())),
                }
                cx.emit(FileOpsEvent::StackChanged);
                cx.notify();
            })
            .ok();
        }));
    }
}
