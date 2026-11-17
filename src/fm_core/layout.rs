//! Pane layout tree (pure): leaves are panes, splits arrange children along an axis.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct PaneId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    SideBySide,
    Stacked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn axis(self) -> Axis {
        match self {
            Side::Left | Side::Right => Axis::SideBySide,
            Side::Top | Side::Bottom => Axis::Stacked,
        }
    }

    /// The new pane goes before (left of / above) the target.
    pub fn leading(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LayoutTree<L> {
    Leaf(L),
    Split { axis: Axis, children: Vec<LayoutTree<L>>, ratios: Vec<f32> },
}

fn equal(n: usize) -> Vec<f32> {
    vec![1.0 / n.max(1) as f32; n]
}

impl<L: Clone + PartialEq> LayoutTree<L> {
    /// Split with equal ratios.
    pub fn split(axis: Axis, children: Vec<LayoutTree<L>>) -> Self {
        let ratios = equal(children.len());
        LayoutTree::Split { axis, children, ratios }
    }

    /// Leaves in reading order (left to right / top to bottom, depth first).
    pub fn leaves(&self) -> Vec<L> {
        match self {
            LayoutTree::Leaf(l) => vec![l.clone()],
            LayoutTree::Split { children, .. } => children.iter().flat_map(|c| c.leaves()).collect(),
        }
    }

    pub fn contains(&self, l: &L) -> bool {
        match self {
            LayoutTree::Leaf(x) => x == l,
            LayoutTree::Split { children, .. } => children.iter().any(|c| c.contains(l)),
        }
    }

    /// Collapses single-child splits, flattens same-axis nesting, drops empty splits, equalizes.
    pub fn normalized(self) -> Option<Self> {
        self.norm().map(Self::equalized)
    }

    fn norm(self) -> Option<Self> {
        match self {
            LayoutTree::Leaf(_) => Some(self),
            LayoutTree::Split { axis, children, .. } => {
                let mut flat: Vec<LayoutTree<L>> = Vec::new();
                for c in children {
                    match c.norm() {
                        None => {}
                        Some(LayoutTree::Split { axis: a, children: g, .. }) if a == axis => flat.extend(g),
                        Some(n) => flat.push(n),
                    }
                }
                match flat.len() {
                    0 => None,
                    1 => flat.pop(),
                    _ => Some(LayoutTree::split(axis, flat)),
                }
            }
        }
    }

    /// Every split gets equal ratios.
    pub fn equalized(self) -> Self {
        match self {
            LayoutTree::Leaf(_) => self,
            LayoutTree::Split { axis, children, .. } => {
                LayoutTree::split(axis, children.into_iter().map(Self::equalized).collect())
            }
        }
    }

    /// Inserts `new` next to `target` on `side`: same-axis parents get a sibling, otherwise the
    /// target is wrapped in a new split. Result is equalized.
    pub fn docking(&self, new: L, target: &L, side: Side) -> Self {
        self.dock(&new, target, side).equalized()
    }

    fn dock(&self, new: &L, target: &L, side: Side) -> Self {
        let node = LayoutTree::Leaf(new.clone());
        match self {
            LayoutTree::Leaf(l) => {
                if l != target {
                    return self.clone();
                }
                let kids = if side.leading() { vec![node, self.clone()] } else { vec![self.clone(), node] };
                LayoutTree::split(side.axis(), kids)
            }
            LayoutTree::Split { axis, children, ratios } => {
                if *axis == side.axis()
                    && let Some(ix) = children.iter().position(|c| matches!(c, LayoutTree::Leaf(l) if l == target))
                {
                    let mut kids = children.clone();
                    kids.insert(if side.leading() { ix } else { ix + 1 }, node);
                    return LayoutTree::split(*axis, kids);
                }
                LayoutTree::Split {
                    axis: *axis,
                    children: children.iter().map(|c| c.dock(new, target, side)).collect(),
                    ratios: ratios.clone(),
                }
            }
        }
    }

    /// Tree without `l` (normalized); None if nothing is left.
    pub fn removing(&self, l: &L) -> Option<Self> {
        self.prune(l).and_then(Self::normalized)
    }

    fn prune(&self, l: &L) -> Option<Self> {
        match self {
            LayoutTree::Leaf(x) => (x != l).then(|| self.clone()),
            LayoutTree::Split { axis, children, ratios } => {
                let kids: Vec<_> = children.iter().filter_map(|c| c.prune(l)).collect();
                if kids.is_empty() {
                    None
                } else {
                    Some(LayoutTree::Split { axis: *axis, children: kids, ratios: ratios.clone() })
                }
            }
        }
    }

    /// Leaf `offset` places from `cur` in reading order, wrapping; None with a single leaf.
    pub fn next_leaf(&self, cur: &L, offset: isize) -> Option<L> {
        let leaves = self.leaves();
        if leaves.len() < 2 {
            return None;
        }
        let i = leaves.iter().position(|x| x == cur)? as isize;
        let n = leaves.len() as isize;
        Some(leaves[(i + offset).rem_euclid(n) as usize].clone())
    }

    /// Moves divider `divider` (between child `divider` and `divider + 1`) of the split at
    /// `split_path` so the first child's share becomes `frac` of the pair's combined share,
    /// clamped so neither side drops below `min_frac` of the whole split. Sum is preserved.
    pub fn resize(&mut self, split_path: &[usize], divider: usize, frac: f32, min_frac: f32) {
        let mut node = self;
        for &ix in split_path {
            match node {
                LayoutTree::Split { children, .. } if ix < children.len() => node = &mut children[ix],
                _ => return,
            }
        }
        let LayoutTree::Split { ratios, .. } = node else { return };
        if divider + 1 >= ratios.len() {
            return;
        }
        let pair = ratios[divider] + ratios[divider + 1];
        // With many panes the pair may be too small for two minimums: shrink the minimum so
        // neither ratio can go negative.
        let m = min_frac.min(pair / 2.0);
        let a = frac.clamp(m, pair - m);
        ratios[divider] = a;
        ratios[divider + 1] = pair - a;
    }

    /// Like `resize`, but `pos` is the divider position as a fraction of the whole split.
    pub fn drag_divider(&mut self, split_path: &[usize], divider: usize, pos: f32, min_frac: f32) {
        let mut node: &LayoutTree<L> = self;
        for &ix in split_path {
            match node {
                LayoutTree::Split { children, .. } if ix < children.len() => node = &children[ix],
                _ => return,
            }
        }
        let LayoutTree::Split { ratios, .. } = node else { return };
        let before: f32 = ratios.iter().take(divider).sum();
        self.resize(split_path, divider, pos - before, min_frac);
    }

    /// Maps leaves, dropping those where `f` returns None; the result is normalized.
    pub fn filter_map_leaves<T: Clone + PartialEq>(&self, mut f: impl FnMut(&L) -> Option<T>) -> Option<LayoutTree<T>> {
        self.fml(&mut f).and_then(LayoutTree::normalized)
    }

    fn fml<T>(&self, f: &mut impl FnMut(&L) -> Option<T>) -> Option<LayoutTree<T>> {
        match self {
            LayoutTree::Leaf(l) => f(l).map(LayoutTree::Leaf),
            LayoutTree::Split { axis, children, ratios } => {
                let mut kids = Vec::new();
                let mut rs = Vec::new();
                for (i, c) in children.iter().enumerate() {
                    if let Some(k) = c.fml(f) {
                        kids.push(k);
                        rs.push(ratios.get(i).copied().unwrap_or(0.0));
                    }
                }
                if kids.is_empty() { None } else { Some(LayoutTree::Split { axis: *axis, children: kids, ratios: rs }) }
            }
        }
    }

    pub fn map<T>(&self, f: &mut impl FnMut(&L) -> T) -> LayoutTree<T> {
        match self {
            LayoutTree::Leaf(l) => LayoutTree::Leaf(f(l)),
            LayoutTree::Split { axis, children, ratios } => LayoutTree::Split {
                axis: *axis,
                children: children.iter().map(|c| c.map(f)).collect(),
                ratios: ratios.clone(),
            },
        }
    }
}

/// Which leaf gets focus after one closed: the one before it, else the new first.
pub fn focus_after_close<L: Clone>(old_leaves: &[L], closed_idx: usize, new_leaves: &[L]) -> L {
    let _ = old_leaves;
    let n = new_leaves.len();
    let ix = closed_idx.saturating_sub(1).min(n.saturating_sub(1));
    new_leaves[ix].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    type T = LayoutTree<u32>;
    fn leaf(n: u32) -> T {
        LayoutTree::Leaf(n)
    }
    fn side(kids: Vec<T>) -> T {
        T::split(Axis::SideBySide, kids)
    }
    fn stacked(kids: Vec<T>) -> T {
        T::split(Axis::Stacked, kids)
    }

    #[test]
    fn dock_right_leaf() {
        let t = leaf(1).docking(2, &1, Side::Right);
        assert_eq!(t, side(vec![leaf(1), leaf(2)]));
        let LayoutTree::Split { ratios, .. } = t else { panic!() };
        assert_eq!(ratios, [0.5, 0.5]);
    }

    #[test]
    fn dock_same_axis_inserts_after_target() {
        let t = leaf(1).docking(2, &1, Side::Right).docking(3, &1, Side::Right);
        assert_eq!(t, side(vec![leaf(1), leaf(3), leaf(2)]));
        let LayoutTree::Split { ratios, .. } = t else { panic!() };
        assert!(ratios.iter().all(|r| (r - 1.0 / 3.0).abs() < 1e-6));
    }

    #[test]
    fn dock_down_in_hsplit_wraps_target() {
        let t = leaf(1).docking(2, &1, Side::Right).docking(3, &2, Side::Bottom);
        assert_eq!(t, side(vec![leaf(1), stacked(vec![leaf(2), leaf(3)])]));
    }

    #[test]
    fn dock_left_and_top_are_leading() {
        assert_eq!(leaf(1).docking(2, &1, Side::Left), side(vec![leaf(2), leaf(1)]));
        assert_eq!(leaf(1).docking(2, &1, Side::Top), stacked(vec![leaf(2), leaf(1)]));
        assert!(Side::Left.leading() && !Side::Right.leading());
    }

    #[test]
    fn remove_collapses_single_child() {
        let t = leaf(1).docking(2, &1, Side::Right);
        assert_eq!(t.removing(&2), Some(leaf(1)));
        let n = leaf(1).docking(2, &1, Side::Right).docking(3, &2, Side::Bottom);
        assert_eq!(n.removing(&3), Some(side(vec![leaf(1), leaf(2)])));
    }

    #[test]
    fn remove_last_none() {
        assert_eq!(leaf(1).removing(&1), None);
    }

    #[test]
    fn normalize_flattens_same_axis() {
        let t = side(vec![leaf(1), side(vec![leaf(2), leaf(3)])]);
        assert_eq!(t.normalized(), Some(side(vec![leaf(1), leaf(2), leaf(3)])));
        let one = stacked(vec![side(vec![leaf(1)])]);
        assert_eq!(one.normalized(), Some(leaf(1)));
    }

    #[test]
    fn leaves_reading_order() {
        let t = side(vec![stacked(vec![leaf(1), leaf(2)]), leaf(3), stacked(vec![leaf(4), leaf(5)])]);
        assert_eq!(t.leaves(), [1, 2, 3, 4, 5]);
        assert!(t.contains(&4) && !t.contains(&9));
    }

    #[test]
    fn next_leaf_wraps() {
        let t = side(vec![leaf(1), leaf(2), leaf(3)]);
        assert_eq!(t.next_leaf(&3, 1), Some(1));
        assert_eq!(t.next_leaf(&1, -1), Some(3));
        assert_eq!(t.next_leaf(&1, 1), Some(2));
        assert_eq!(leaf(1).next_leaf(&1, 1), None);
    }

    #[test]
    fn focus_after_close_rules() {
        let old = [1, 2, 3];
        assert_eq!(focus_after_close(&old, 0, &[2, 3]), 2);
        assert_eq!(focus_after_close(&old, 1, &[1, 3]), 1);
        assert_eq!(focus_after_close(&old, 2, &[1, 2]), 2);
    }

    #[test]
    fn resize_clamps_min_and_preserves_sum() {
        let mut t = side(vec![leaf(1), leaf(2), leaf(3)]);
        t.resize(&[], 0, 0.5, 0.1);
        let LayoutTree::Split { ratios, .. } = &t else { panic!() };
        assert!((ratios[0] - 0.5).abs() < 1e-6);
        assert!((ratios.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        t.resize(&[], 0, 0.0, 0.1);
        let LayoutTree::Split { ratios, .. } = &t else { panic!() };
        assert!((ratios[0] - 0.1).abs() < 1e-6);
        t.resize(&[], 1, 5.0, 0.1);
        let LayoutTree::Split { ratios, .. } = &t else { panic!() };
        assert!((ratios[2] - 0.1).abs() < 1e-6);
        assert!((ratios.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn resize_never_goes_negative_with_many_panes() {
        let mut t = side((1..=6).map(leaf).collect());
        t.resize(&[], 2, 0.0, 0.45);
        t.resize(&[], 2, 9.0, 0.45);
        let LayoutTree::Split { ratios, .. } = &t else { panic!() };
        assert!(ratios.iter().all(|r| *r > 0.0), "{ratios:?}");
        assert!((ratios.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn drag_divider_uses_whole_split_position() {
        let mut t = side(vec![leaf(1), leaf(2), leaf(3)]);
        t.drag_divider(&[], 1, 0.5, 0.1);
        let LayoutTree::Split { ratios, .. } = &t else { panic!() };
        assert!((ratios[0] - 1.0 / 3.0).abs() < 1e-6);
        assert!((ratios[1] - (0.5 - 1.0 / 3.0)).abs() < 1e-6);
        assert!((ratios.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        let mut nested = side(vec![leaf(1), stacked(vec![leaf(2), leaf(3)])]);
        nested.drag_divider(&[1], 0, 0.7, 0.1);
        let LayoutTree::Split { children, .. } = &nested else { panic!() };
        let LayoutTree::Split { ratios, .. } = &children[1] else { panic!() };
        assert!((ratios[0] - 0.7).abs() < 1e-6);
    }

    #[test]
    fn map_keeps_shape() {
        let t = side(vec![leaf(1), leaf(2)]).map(&mut |x| x * 10);
        assert_eq!(t, LayoutTree::split(Axis::SideBySide, vec![LayoutTree::Leaf(10), LayoutTree::Leaf(20)]));
    }

    #[test]
    fn filter_map_drops_and_normalizes() {
        let t: T = LayoutTree::split(Axis::Stacked, vec![
            LayoutTree::split(Axis::SideBySide, vec![leaf(1), leaf(2)]),
            LayoutTree::split(Axis::SideBySide, vec![leaf(3), leaf(4)]),
        ]);
        let r = t.filter_map_leaves(|l| (*l != 2 && *l != 4).then_some(*l * 10)).unwrap();
        assert_eq!(r, LayoutTree::split(Axis::Stacked, vec![LayoutTree::Leaf(10), LayoutTree::Leaf(30)]));
        assert!(t.filter_map_leaves(|_| None::<u32>).is_none());
    }

    #[test]
    fn serde_round_trip() {
        let t: T = LayoutTree::split(Axis::Stacked, vec![leaf(1), leaf(2)]);
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<T>(&j).unwrap(), t);
    }
}
