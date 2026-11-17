use super::ops::{OpError, UndoRecord, reapply, revert};

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<UndoRecord>,
    redo: Vec<UndoRecord>,
}

impl UndoStack {
    /// A new operation invalidates the redo history.
    pub fn push(&mut self, rec: UndoRecord) {
        self.undo.push(rec);
        self.redo.clear();
    }

    pub fn undo_title(&self) -> Option<&str> {
        self.undo.last().map(|r| r.title.as_str())
    }

    pub fn redo_title(&self) -> Option<&str> {
        self.redo.last().map(|r| r.title.as_str())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn peek_undo(&self) -> Option<UndoRecord> {
        self.undo.last().cloned()
    }

    pub fn peek_redo(&self) -> Option<UndoRecord> {
        self.redo.last().cloned()
    }

    /// Ok: move the (updated) record to redo. SourceMissing: drop it. Other errors: keep it.
    pub fn finish_undo(&mut self, updated: UndoRecord, res: &Result<(), OpError>) {
        match res {
            Ok(()) => {
                self.undo.pop();
                self.redo.push(updated);
            }
            Err(OpError::SourceMissing(_)) => {
                self.undo.pop();
            }
            Err(_) => {}
        }
    }

    /// Ok: move the record to undo (redo is not cleared). Same error rules as undo.
    pub fn finish_redo(&mut self, updated: UndoRecord, res: &Result<(), OpError>) {
        match res {
            Ok(()) => {
                self.redo.pop();
                self.undo.push(updated);
            }
            Err(OpError::SourceMissing(_)) => {
                self.redo.pop();
            }
            Err(_) => {}
        }
    }

    /// Synchronous convenience (tests). Nothing to undo is a no-op.
    pub fn undo(&mut self) -> Result<(), OpError> {
        let Some(mut rec) = self.peek_undo() else { return Ok(()) };
        let res = revert(&mut rec);
        self.finish_undo(rec, &res);
        res
    }

    pub fn redo(&mut self) -> Result<(), OpError> {
        let Some(mut rec) = self.peek_redo() else { return Ok(()) };
        let res = reapply(&mut rec);
        self.finish_redo(rec, &res);
        res
    }
}
