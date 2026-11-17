//! Ordered tab list with an active index (pure).

#[derive(Clone, Debug)]
pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Tabs<T> {
    pub fn new(first: T) -> Self {
        Self { items: vec![first], active: 0 }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_item(&self) -> &T {
        &self.items[self.active]
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Inserts right after the active tab and selects it.
    pub fn add_after_active(&mut self, t: T) {
        self.active += 1;
        self.items.insert(self.active, t);
    }

    /// Removes tab `i`; None (and no change) if it is the only tab or out of range.
    pub fn close(&mut self, i: usize) -> Option<T> {
        if self.items.len() < 2 || i >= self.items.len() {
            return None;
        }
        let t = self.items.remove(i);
        if i < self.active || self.active >= self.items.len() {
            self.active -= 1;
        }
        Some(t)
    }

    pub fn select(&mut self, i: usize) {
        if i < self.items.len() {
            self.active = i;
        }
    }

    pub fn cycle(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(n) as usize;
    }

    /// Moves a tab; the active tab stays the same item.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return;
        }
        let t = self.items.remove(from);
        self.items.insert(to, t);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            self.active += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abc() -> Tabs<char> {
        let mut t = Tabs::new('a');
        t.add_after_active('b');
        t.select(0);
        t
    }

    #[test]
    fn add_inserts_after_active_and_selects() {
        let mut t = abc();
        t.add_after_active('n');
        assert_eq!(t.items(), ['a', 'n', 'b']);
        assert_eq!(t.active(), 1);
    }

    #[test]
    fn close_active_middle_keeps_index() {
        let mut t = Tabs::new('a');
        for c in ['b', 'c'] {
            t.add_after_active(c);
        }
        t.select(1);
        assert_eq!(t.close(1), Some('b'));
        assert_eq!(t.active(), 1);
        assert_eq!(*t.active_item(), 'c');
    }

    #[test]
    fn close_active_last_goes_previous() {
        let mut t = abc();
        t.select(1);
        t.close(1);
        assert_eq!(t.active(), 0);
    }

    #[test]
    fn close_left_of_active_shifts() {
        let mut t = Tabs::new('a');
        for c in ['b', 'c'] {
            t.add_after_active(c);
        }
        assert_eq!(t.active(), 2);
        t.close(0);
        assert_eq!(t.active(), 1);
        assert_eq!(*t.active_item(), 'c');
    }

    #[test]
    fn close_only_is_none() {
        let mut t = Tabs::new('a');
        assert_eq!(t.close(0), None);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn cycle_wraps() {
        let mut t = Tabs::new('a');
        for c in ['b', 'c'] {
            t.add_after_active(c);
        }
        t.cycle(1);
        assert_eq!(t.active(), 0);
        t.cycle(-1);
        assert_eq!(t.active(), 2);
    }

    #[test]
    fn move_keeps_active_item() {
        let mut t = Tabs::new('a');
        for c in ['b', 'c'] {
            t.add_after_active(c);
        }
        t.select(1);
        t.move_tab(0, 2);
        assert_eq!(t.items(), ['b', 'c', 'a']);
        assert_eq!(*t.active_item(), 'b');
        t.move_tab(0, 1);
        assert_eq!(*t.active_item(), 'b');
    }
}
