//! The panes of a tab, as tmux splits them: a split cuts one pane in two, side by side or
//! one above the other. Generic over the leaf, so the saved layout (addresses) and the
//! live one (views) share the same tree code.

use serde::{Deserialize, Serialize};

/// tmux's names: `-h` puts the panes side by side, `-v` one above the other.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Horizontal,
    Vertical,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Node<L> {
    Leaf(L),
    Split {
        split: Dir,
        /// The first pane's share, 0..1.
        ratio: f32,
        a: Box<Node<L>>,
        b: Box<Node<L>>,
    },
}

/// A pane's place in the tab, in fractions of the tab's area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

const FULL: Rect = Rect {
    x: 0.,
    y: 0.,
    w: 1.,
    h: 1.,
};

/// tmux's select-pane `-L` `-R` `-U` `-D`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// Slack for edges that meet after float arithmetic.
const EPS: f32 = 1e-4;

impl<L> Node<L> {
    /// In tmux's pane order: left before right, top before bottom.
    pub fn leaves(&self) -> Vec<&L> {
        let mut out = Vec::new();
        self.walk(&mut |l| out.push(l));
        out
    }

    fn walk<'a>(&'a self, f: &mut impl FnMut(&'a L)) {
        match self {
            Node::Leaf(l) => f(l),
            Node::Split { a, b, .. } => {
                a.walk(f);
                b.walk(f);
            }
        }
    }

    pub fn find(&self, is: &impl Fn(&L) -> bool) -> Option<&L> {
        self.leaves().into_iter().find(|l| is(l))
    }

    pub fn contains(&self, is: &impl Fn(&L) -> bool) -> bool {
        self.find(is).is_some()
    }

    /// Splits the leaf `is` picks: it keeps the first half, `new` gets the second.
    /// `new` comes back when no leaf matched.
    pub fn split(self, is: &impl Fn(&L) -> bool, dir: Dir, new: L) -> (Node<L>, Option<L>) {
        match self {
            Node::Leaf(l) if is(&l) => (
                Node::Split {
                    split: dir,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(l)),
                    b: Box::new(Node::Leaf(new)),
                },
                None,
            ),
            leaf @ Node::Leaf(_) => (leaf, Some(new)),
            Node::Split { split, ratio, a, b } => {
                let (a, rest) = a.split(is, dir, new);
                let (b, rest) = match rest {
                    Some(new) => b.split(is, dir, new),
                    None => (*b, None),
                };
                (
                    Node::Split {
                        split,
                        ratio,
                        a: Box::new(a),
                        b: Box::new(b),
                    },
                    rest,
                )
            }
        }
    }

    /// Takes out the leaf `is` picks; its sibling takes the space. `None` when the tree
    /// was that one leaf, else the tree without it (unchanged if no leaf matched).
    pub fn remove(self, is: &impl Fn(&L) -> bool) -> Option<Node<L>> {
        match self {
            Node::Leaf(l) if is(&l) => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { split, ratio, a, b } => match (a.remove(is), b.remove(is)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    split,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// Keeps the leaves `keep` accepts, closing up the splits around the others.
    pub fn retain(self, keep: &impl Fn(&L) -> bool) -> Option<Node<L>> {
        self.remove(&|l: &L| !keep(l))
    }

    /// Every leaf with its place in the tab.
    pub fn rects(&self) -> Vec<(&L, Rect)> {
        let mut out = Vec::new();
        self.rects_in(FULL, &mut out);
        out
    }

    fn rects_in<'a>(&'a self, r: Rect, out: &mut Vec<(&'a L, Rect)>) {
        match self {
            Node::Leaf(l) => out.push((l, r)),
            Node::Split { split, ratio, a, b } => {
                let (ra, rb) = match split {
                    Dir::Horizontal => (
                        Rect { w: r.w * ratio, ..r },
                        Rect { x: r.x + r.w * ratio, w: r.w * (1. - ratio), ..r },
                    ),
                    Dir::Vertical => (
                        Rect { h: r.h * ratio, ..r },
                        Rect { y: r.y + r.h * ratio, h: r.h * (1. - ratio), ..r },
                    ),
                };
                a.rects_in(ra, out);
                b.rects_in(rb, out);
            }
        }
    }

    /// The pane on that side of the one `is` picks, as tmux's select-pane: the touching
    /// pane that overlaps it most, else (at the edge) the farthest one on the other side.
    pub fn neighbor(&self, is: &impl Fn(&L) -> bool, side: Side) -> Option<&L> {
        let rects = self.rects();
        let (_, r) = *rects.iter().find(|(l, _)| is(l))?;
        // Overlap across the direction of travel, and position along it
        let overlap = |c: &Rect| match side {
            Side::Left | Side::Right => (c.y + c.h).min(r.y + r.h) - c.y.max(r.y),
            Side::Up | Side::Down => (c.x + c.w).min(r.x + r.w) - c.x.max(r.x),
        };
        let touches = |c: &Rect| match side {
            Side::Left => (c.x + c.w - r.x).abs() < EPS,
            Side::Right => (r.x + r.w - c.x).abs() < EPS,
            Side::Up => (c.y + c.h - r.y).abs() < EPS,
            Side::Down => (r.y + r.h - c.y).abs() < EPS,
        };
        let far = |c: &Rect| match side {
            Side::Left => c.x,
            Side::Right => -c.x,
            Side::Up => c.y,
            Side::Down => -c.y,
        };
        // The best touching pane by overlap, and the best one to wrap to by distance
        let mut near: Option<(&L, f32)> = None;
        let mut wrap: Option<(&L, f32)> = None;
        for (l, c) in &rects {
            if is(l) || overlap(c) <= EPS {
                continue;
            }
            if touches(c) && near.is_none_or(|(_, o)| overlap(c) > o) {
                near = Some((*l, overlap(c)));
            }
            if wrap.is_none_or(|(_, f)| far(c) > f) {
                wrap = Some((*l, far(c)));
            }
        }
        near.or(wrap).map(|(l, _)| l)
    }

    pub fn map<M>(&self, f: &impl Fn(&L) -> M) -> Node<M> {
        match self {
            Node::Leaf(l) => Node::Leaf(f(l)),
            Node::Split { split, ratio, a, b } => Node::Split {
                split: *split,
                ratio: *ratio,
                a: Box::new(a.map(f)),
                b: Box::new(b.map(f)),
            },
        }
    }

    pub fn try_map<M>(self, f: &mut impl FnMut(L) -> Option<M>) -> Option<Node<M>> {
        match self {
            Node::Leaf(l) => f(l).map(Node::Leaf),
            Node::Split { split, ratio, a, b } => match (a.try_map(f), b.try_map(f)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    split,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(s: &str) -> Node<String> {
        Node::Leaf(s.to_string())
    }

    fn is(s: &'static str) -> impl Fn(&String) -> bool {
        move |l: &String| l == s
    }

    fn split(n: Node<String>, at: &'static str, dir: Dir, new: &str) -> Node<String> {
        let (n, rest) = n.split(&is(at), dir, new.into());
        assert_eq!(rest, None);
        n
    }

    #[test]
    fn split_keeps_the_old_pane_first() {
        let n = split(leaf("t1"), "t1", Dir::Horizontal, "t2");
        let n = split(n, "t2", Dir::Vertical, "t3");
        assert_eq!(n.leaves(), ["t1", "t2", "t3"]);
        let (_, rest) = n.split(&is("t9"), Dir::Vertical, "t4".into());
        assert_eq!(rest.as_deref(), Some("t4"));
    }

    #[test]
    fn rects_follow_the_splits() {
        let n = split(leaf("t1"), "t1", Dir::Horizontal, "t2");
        let n = split(n, "t2", Dir::Vertical, "t3");
        let rects = n.rects();
        assert_eq!(rects[0].1, Rect { x: 0., y: 0., w: 0.5, h: 1. });
        assert_eq!(rects[2].1, Rect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
    }

    #[test]
    fn neighbors_touch_or_wrap() {
        // t1 | t2
        //    | t3
        let n = split(leaf("t1"), "t1", Dir::Horizontal, "t2");
        let n = split(n, "t2", Dir::Vertical, "t3");
        let at = |a: &'static str, side| n.neighbor(&is(a), side).map(String::as_str);
        assert_eq!(at("t1", Side::Right), Some("t2"));
        assert_eq!(at("t3", Side::Left), Some("t1"));
        assert_eq!(at("t3", Side::Up), Some("t2"));
        assert_eq!(at("t2", Side::Down), Some("t3"));
        // At the edge it wraps to the far side
        assert_eq!(at("t1", Side::Left), Some("t2"));
        assert_eq!(at("t2", Side::Up), Some("t3"));
        // Nothing above or below t1
        assert_eq!(at("t1", Side::Up), None);
    }

    #[test]
    fn remove_lets_the_sibling_take_the_space() {
        let n = split(leaf("t1"), "t1", Dir::Horizontal, "t2");
        let n = split(n, "t2", Dir::Vertical, "t3");
        let n = n.remove(&is("t2")).unwrap();
        assert_eq!(n.leaves(), ["t1", "t3"]);
        assert!(matches!(n, Node::Split { split: Dir::Horizontal, .. }));
        let n = n.remove(&is("t1")).unwrap();
        assert_eq!(n, leaf("t3"));
        assert_eq!(n.remove(&is("t3")), None);
    }

    #[test]
    fn saved_form_is_an_address_or_a_split() {
        let n = split(leaf("t1"), "t1", Dir::Horizontal, "t2");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"split": "horizontal", "ratio": 0.5, "a": "t1", "b": "t2"})
        );
        assert_eq!(serde_json::from_value::<Node<String>>(v).unwrap(), n);
        assert_eq!(serde_json::from_value::<Node<String>>("t1".into()).unwrap(), leaf("t1"));
    }
}
