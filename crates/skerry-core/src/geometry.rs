//! Screen geometry: displays, desktop bounds, edge crossing and entry points.
//!
//! Every machine describes its desktop as a list of display rectangles in its
//! own coordinate space. Edges are the outer boundary of the bounding box of
//! all displays: a cursor leaves through the right edge when it is pushed past
//! the right-most column of pixels, and so on.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

    pub fn opposite(self) -> Edge {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Edge::Left => 0,
            Edge::Right => 1,
            Edge::Top => 2,
            Edge::Bottom => 3,
        }
    }

    /// True for edges crossed by horizontal movement (left/right).
    pub fn is_vertical_line(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }

    pub fn name(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<Edge> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            "top" | "up" => Some(Edge::Top),
            "bottom" | "down" => Some(Edge::Bottom),
            _ => None,
        }
    }
}

/// A small set of edges, stored as a bitmask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeSet(u8);

impl EdgeSet {
    pub const fn empty() -> Self {
        EdgeSet(0)
    }
    pub fn from_bits(bits: u8) -> Self {
        EdgeSet(bits & 0x0f)
    }
    pub fn bits(self) -> u8 {
        self.0
    }
    pub fn insert(&mut self, e: Edge) {
        self.0 |= 1 << e.index();
    }
    pub fn contains(self, e: Edge) -> bool {
        self.0 & (1 << e.index()) != 0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn iter(self) -> impl Iterator<Item = Edge> {
        Edge::ALL.into_iter().filter(move |e| self.contains(*e))
    }
}

impl FromIterator<Edge> for EdgeSet {
    fn from_iter<T: IntoIterator<Item = Edge>>(iter: T) -> Self {
        let mut s = EdgeSet::empty();
        for e in iter {
            s.insert(e);
        }
        s
    }
}

/// An axis-aligned rectangle. `x + w` and `y + h` are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && x < self.right() as f64 && y >= self.y as f64 && y < self.bottom() as f64
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(x, y, r - x, b - y)
    }
    /// Clamp a point into this rectangle (inclusive of the last pixel).
    pub fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        let maxx = (self.right() - 1) as f64;
        let maxy = (self.bottom() - 1) as f64;
        (x.clamp(self.x as f64, maxx.max(self.x as f64)), y.clamp(self.y as f64, maxy.max(self.y as f64)))
    }
    fn distance_sq(&self, x: f64, y: f64) -> f64 {
        let (cx, cy) = self.clamp(x, y);
        (cx - x).powi(2) + (cy - y).powi(2)
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x as f64 + self.w as f64 / 2.0, self.y as f64 + self.h as f64 / 2.0)
    }
}

/// Result of moving a virtual cursor across a desktop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// The cursor stays on this desktop at the given position.
    Inside(f64, f64),
    /// The cursor was pushed through an outer edge. `frac` is the position
    /// along that edge (0.0 = top/left end, 1.0 = bottom/right end) and
    /// `clamped` is where it would stop if nothing is on the other side.
    Exit { edge: Edge, frac: f64, clamped: (f64, f64) },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Desktop {
    pub displays: Vec<Rect>,
}

impl Desktop {
    pub fn new(displays: Vec<Rect>) -> Self {
        Desktop { displays: displays.into_iter().filter(|r| !r.is_empty()).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.displays.is_empty()
    }

    pub fn bounds(&self) -> Rect {
        let mut it = self.displays.iter();
        match it.next() {
            None => Rect::new(0, 0, 1, 1),
            Some(first) => it.fold(*first, |acc, r| acc.union(r)),
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.displays.iter().any(|d| d.contains(x, y))
    }

    pub fn display_at(&self, x: f64, y: f64) -> Option<&Rect> {
        self.displays.iter().find(|d| d.contains(x, y))
    }

    fn nearest_display(&self, x: f64, y: f64) -> Option<&Rect> {
        self.displays.iter().min_by(|a, b| a.distance_sq(x, y).total_cmp(&b.distance_sq(x, y)))
    }

    /// Clamp a point onto the closest display.
    pub fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        if self.contains(x, y) {
            return (x, y);
        }
        match self.nearest_display(x, y) {
            Some(d) => d.clamp(x, y),
            None => (x, y),
        }
    }

    pub fn center(&self) -> (f64, f64) {
        match self.displays.first() {
            Some(d) => d.center(),
            None => (0.0, 0.0),
        }
    }

    /// Position along an edge as a fraction of the desktop's extent.
    pub fn fraction(&self, edge: Edge, x: f64, y: f64) -> f64 {
        let b = self.bounds();
        let f = if edge.is_vertical_line() {
            (y - b.y as f64) / (b.h.max(1) as f64)
        } else {
            (x - b.x as f64) / (b.w.max(1) as f64)
        };
        f.clamp(0.0, 1.0)
    }

    /// Where a cursor arriving through `edge` of this desktop at `frac` lands.
    pub fn entry_point(&self, edge: Edge, frac: f64) -> (f64, f64) {
        let b = self.bounds();
        let frac = frac.clamp(0.0, 1.0);
        if self.displays.is_empty() {
            return b.center();
        }
        if edge.is_vertical_line() {
            let ty = (b.y as f64 + frac * b.h as f64).min((b.bottom() - 1) as f64);
            let covering: Vec<&Rect> =
                self.displays.iter().filter(|d| ty >= d.y as f64 && ty < d.bottom() as f64).collect();
            let pick = |cands: &[&Rect]| -> Option<Rect> {
                match edge {
                    Edge::Left => cands.iter().min_by_key(|d| d.x).map(|d| **d),
                    _ => cands.iter().max_by_key(|d| d.right()).map(|d| **d),
                }
            };
            let d = pick(&covering).or_else(|| self.nearest_display(b.center().0, ty).copied()).unwrap();
            let (_, cy) = d.clamp(0.0, ty);
            let x = if edge == Edge::Left { d.x as f64 } else { (d.right() - 1) as f64 };
            (x, cy)
        } else {
            let tx = (b.x as f64 + frac * b.w as f64).min((b.right() - 1) as f64);
            let covering: Vec<&Rect> =
                self.displays.iter().filter(|d| tx >= d.x as f64 && tx < d.right() as f64).collect();
            let d = match edge {
                Edge::Top => covering.iter().min_by_key(|d| d.y).map(|d| **d),
                _ => covering.iter().max_by_key(|d| d.bottom()).map(|d| **d),
            }
            .or_else(|| self.nearest_display(tx, b.center().1).copied())
            .unwrap();
            let (cx, _) = d.clamp(tx, 0.0);
            let y = if edge == Edge::Top { d.y as f64 } else { (d.bottom() - 1) as f64 };
            (cx, y)
        }
    }

    /// Move an entry point a few pixels inward so the cursor does not
    /// immediately re-trigger the edge it arrived through.
    pub fn inset(&self, edge: Edge, (x, y): (f64, f64), px: f64) -> (f64, f64) {
        let p = match edge {
            Edge::Left => (x + px, y),
            Edge::Right => (x - px, y),
            Edge::Top => (x, y + px),
            Edge::Bottom => (x, y - px),
        };
        self.clamp(p.0, p.1)
    }

    /// The outer edge a pixel position touches, if any. Used by capture
    /// backends that see absolute cursor positions.
    pub fn edge_at(&self, x: f64, y: f64) -> Option<Edge> {
        self.edge_near(x, y, 0.0)
    }

    /// Like [`Desktop::edge_at`], but also counts positions within `slack`
    /// pixels of an edge (for systems that report fractional positions and
    /// may stop the cursor just short of the last pixel).
    pub fn edge_near(&self, x: f64, y: f64, slack: f64) -> Option<Edge> {
        if self.displays.is_empty() {
            return None;
        }
        let b = self.bounds();
        if x <= b.x as f64 + slack {
            Some(Edge::Left)
        } else if x >= (b.right() - 1) as f64 - slack {
            Some(Edge::Right)
        } else if y <= b.y as f64 + slack {
            Some(Edge::Top)
        } else if y >= (b.bottom() - 1) as f64 - slack {
            Some(Edge::Bottom)
        } else {
            None
        }
    }

    /// Move a virtual cursor from `from` by `(dx, dy)`.
    pub fn step(&self, from: (f64, f64), dx: f64, dy: f64) -> Step {
        let to = (from.0 + dx, from.1 + dy);
        if self.contains(to.0, to.1) {
            return Step::Inside(to.0, to.1);
        }
        let b = self.bounds();
        let over_r = to.0 - (b.right() - 1) as f64;
        let over_l = b.x as f64 - to.0;
        let over_b = to.1 - (b.bottom() - 1) as f64;
        let over_t = b.y as f64 - to.1;

        let mut best: Option<(Edge, f64)> = None;
        for (edge, over, moving) in [
            (Edge::Right, over_r, dx > 0.0),
            (Edge::Left, over_l, dx < 0.0),
            (Edge::Bottom, over_b, dy > 0.0),
            (Edge::Top, over_t, dy < 0.0),
        ] {
            if moving && over > 0.0 && best.is_none_or(|(_, o)| over > o) {
                best = Some((edge, over));
            }
        }

        let clamped = self.clamp_move(from, to);
        match best {
            Some((edge, _)) => {
                let frac = self.fraction(edge, to.0, to.1);
                Step::Exit { edge, frac, clamped }
            }
            None => Step::Inside(clamped.0, clamped.1),
        }
    }

    /// Clamp a movement that left the desktop: slide along whichever axis is
    /// still valid, otherwise stop at the boundary of the current display.
    fn clamp_move(&self, from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
        if self.contains(to.0, from.1) {
            let d = self.display_at(to.0, from.1).unwrap();
            return d.clamp(to.0, to.1);
        }
        if self.contains(from.0, to.1) {
            let d = self.display_at(from.0, to.1).unwrap();
            return d.clamp(to.0, to.1);
        }
        match self.display_at(from.0, from.1).or_else(|| self.nearest_display(from.0, from.1)) {
            Some(d) => d.clamp(to.0, to.1),
            None => to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single() -> Desktop {
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080)])
    }

    fn dual() -> Desktop {
        // Main 1920x1080 plus a smaller 1280x1024 to its right, top-aligned.
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 1024)])
    }

    #[test]
    fn inside_moves() {
        assert_eq!(single().step((100.0, 100.0), 5.0, -3.0), Step::Inside(105.0, 97.0));
    }

    #[test]
    fn exit_right_with_fraction() {
        match single().step((1919.0, 540.0), 4.0, 0.0) {
            Step::Exit { edge, frac, .. } => {
                assert_eq!(edge, Edge::Right);
                assert!((frac - 0.5).abs() < 0.01);
            }
            s => panic!("unexpected {s:?}"),
        }
    }

    #[test]
    fn edge_with_slack() {
        let d = single();
        assert_eq!(d.edge_at(1918.4, 500.0), None);
        assert_eq!(d.edge_near(1918.4, 500.0, 1.0), Some(Edge::Right));
        assert_eq!(d.edge_near(0.6, 500.0, 1.0), Some(Edge::Left));
        assert_eq!(d.edge_near(900.0, 500.0, 1.0), None);
    }

    #[test]
    fn exit_top_and_left() {
        assert!(matches!(single().step((10.0, 0.0), 0.0, -2.0), Step::Exit { edge: Edge::Top, .. }));
        assert!(matches!(single().step((0.0, 10.0), -1.0, 0.0), Step::Exit { edge: Edge::Left, .. }));
    }

    #[test]
    fn crossing_between_local_displays_stays_inside() {
        assert_eq!(dual().step((1919.0, 500.0), 10.0, 0.0), Step::Inside(1929.0, 500.0));
    }

    #[test]
    fn dead_zone_below_smaller_display_clamps() {
        // From the main display at y=1050 moving right: the second display
        // does not extend that low, and this is not the desktop's right edge.
        match dual().step((1919.0, 1050.0), 10.0, 0.0) {
            Step::Inside(x, y) => {
                assert_eq!(x, 1919.0);
                assert_eq!(y, 1050.0);
            }
            s => panic!("unexpected {s:?}"),
        }
    }

    #[test]
    fn entry_points() {
        let d = dual();
        assert_eq!(d.entry_point(Edge::Left, 0.5), (0.0, 540.0));
        let (x, y) = d.entry_point(Edge::Right, 0.25);
        assert_eq!(x, 3199.0);
        assert_eq!(y, 270.0);
        // Right edge below the smaller display falls back to the nearest display.
        let (x, y) = d.entry_point(Edge::Right, 0.99);
        assert!(y <= 1079.0);
        assert!(d.contains(x, y));
        let (x, y) = d.entry_point(Edge::Top, 0.0);
        assert_eq!((x, y), (0.0, 0.0));
        let (x, y) = d.entry_point(Edge::Bottom, 0.9);
        assert!(d.contains(x, y));
        assert_eq!(y, 1023.0);
    }

    #[test]
    fn edge_at_detects_outer_boundary() {
        let d = dual();
        assert_eq!(d.edge_at(3199.0, 10.0), Some(Edge::Right));
        assert_eq!(d.edge_at(1919.0, 10.0), None);
        assert_eq!(d.edge_at(0.0, 10.0), Some(Edge::Left));
        assert_eq!(d.edge_at(500.0, 1079.0), Some(Edge::Bottom));
    }

    #[test]
    fn negative_coordinates() {
        // Secondary display to the left of the primary (Windows/macOS style).
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(-1440, 0, 1440, 900)]);
        assert_eq!(d.bounds(), Rect::new(-1440, 0, 3360, 1080));
        assert!(matches!(d.step((-1440.0, 100.0), -1.0, 0.0), Step::Exit { edge: Edge::Left, .. }));
        assert_eq!(d.entry_point(Edge::Left, 0.5), (-1440.0, 540.0));
    }

    #[test]
    fn edgeset() {
        let s: EdgeSet = [Edge::Left, Edge::Bottom].into_iter().collect();
        assert!(s.contains(Edge::Left));
        assert!(!s.contains(Edge::Right));
        assert_eq!(s.iter().count(), 2);
    }
}
