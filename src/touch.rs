//! Touch gesture recognition, kept free of Win32 so it can be unit tested.
//! Coordinates are client pixels; times are milliseconds.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Move content by (dx, dy).
    Pan(f32, f32),
    /// Two-finger pinch: the content point under `from` moves to `to` while scaling by `factor`.
    /// One combined transform so translation and scaling are clamped together.
    Pinch {
        factor: f32,
        from: (f32, f32),
        to: (f32, f32),
    },
    Tap(f32, f32),
    DoubleTap(f32, f32),
    /// Horizontal flick: +1 = next (finger moved left), -1 = previous.
    Swipe(i32),
}

#[derive(Clone, Copy)]
struct Point {
    id: u32,
    x: f32,
    y: f32,
}

pub struct Recognizer {
    points: Vec<Point>,
    start: (f32, f32),
    /// Finger moved beyond the slop, so this can't be a tap.
    moved: bool,
    /// A second finger was involved at some point in this gesture.
    multi: bool,
    /// Single-finger movement was used to pan, so it isn't a swipe.
    panned: bool,
    last_tap: Option<(u64, f32, f32)>,
    /// Whether single-finger drags should pan (content larger than the viewport). Set by the caller.
    pub can_pan: bool,
    pub slop: f32,
    pub swipe_distance: f32,
    pub double_tap_ms: u64,
}

impl Recognizer {
    pub fn new(scale: f32) -> Self {
        let mut r = Recognizer {
            points: Vec::new(),
            start: (0.0, 0.0),
            moved: false,
            multi: false,
            panned: false,
            last_tap: None,
            can_pan: false,
            slop: 0.0,
            swipe_distance: 0.0,
            double_tap_ms: 350,
        };
        r.set_scale(scale);
        r
    }

    /// Thresholds are defined in DIPs and scaled to pixels.
    pub fn set_scale(&mut self, scale: f32) {
        self.slop = 8.0 * scale;
        self.swipe_distance = 50.0 * scale;
    }

    pub fn active(&self) -> bool {
        !self.points.is_empty()
    }

    pub fn finger_count(&self) -> usize {
        self.points.len()
    }

    pub fn cancel(&mut self) {
        self.points.clear();
        self.last_tap = None;
    }

    pub fn down(&mut self, id: u32, x: f32, y: f32) -> Vec<Action> {
        if self.points.is_empty() {
            self.start = (x, y);
            self.moved = false;
            self.multi = false;
            self.panned = false;
        } else {
            self.multi = true;
            self.moved = true;
        }
        self.points.retain(|p| p.id != id);
        self.points.push(Point { id, x, y });
        Vec::new()
    }

    pub fn update(&mut self, id: u32, x: f32, y: f32) -> Vec<Action> {
        let Some(i) = self.points.iter().position(|p| p.id == id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if self.points.len() >= 2 && i < 2 {
            // Pinch with the first two fingers: follow the midpoint and scale by the spread.
            let (a, b) = (self.points[0], self.points[1]);
            let (mut na, mut nb) = (a, b);
            if i == 0 {
                na = Point { x, y, ..a }
            } else {
                nb = Point { x, y, ..b }
            }
            let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            let (nmx, nmy) = ((na.x + nb.x) / 2.0, (na.y + nb.y) / 2.0);
            let d0 = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
            let d1 = ((na.x - nb.x).powi(2) + (na.y - nb.y).powi(2)).sqrt();
            let factor = if d0 > 1.0 && d1 > 1.0 { d1 / d0 } else { 1.0 };
            if factor != 1.0 || nmx != mx || nmy != my {
                out.push(Action::Pinch {
                    factor,
                    from: (mx, my),
                    to: (nmx, nmy),
                });
            }
        } else if self.points.len() == 1 {
            let p = self.points[0];
            if !self.moved
                && ((x - self.start.0).powi(2) + (y - self.start.1).powi(2)).sqrt() > self.slop
            {
                self.moved = true;
                if self.can_pan || self.multi {
                    // Include the motion inside the slop so content doesn't lag the finger.
                    self.panned = true;
                    out.push(Action::Pan(x - self.start.0, y - self.start.1));
                }
            } else if self.moved && (self.can_pan || self.multi) {
                self.panned = true;
                out.push(Action::Pan(x - p.x, y - p.y));
            }
        }
        self.points[i].x = x;
        self.points[i].y = y;
        out
    }

    pub fn up(&mut self, id: u32, x: f32, y: f32, now_ms: u64) -> Vec<Action> {
        // An up for a finger we aren't tracking (e.g. after cancel) must not act on stale state.
        if !self.points.iter().any(|p| p.id == id) {
            return Vec::new();
        }
        let mut out = self.update(id, x, y);
        self.points.retain(|p| p.id != id);
        if !self.points.is_empty() {
            return out;
        }
        let (dx, dy) = (x - self.start.0, y - self.start.1);
        if !self.moved && !self.multi {
            let double = self.last_tap.is_some_and(|(t, tx, ty)| {
                now_ms.saturating_sub(t) <= self.double_tap_ms
                    && ((x - tx).powi(2) + (y - ty).powi(2)).sqrt() <= self.slop * 3.0
            });
            if double {
                self.last_tap = None;
                out.push(Action::DoubleTap(x, y));
            } else {
                self.last_tap = Some((now_ms, x, y));
                out.push(Action::Tap(x, y));
            }
        } else if !self.multi
            && !self.panned
            && dx.abs() > self.swipe_distance
            && dx.abs() > dy.abs() * 1.5
        {
            out.push(Action::Swipe(if dx < 0.0 { 1 } else { -1 }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(can_pan: bool) -> Recognizer {
        let mut r = Recognizer::new(1.0);
        r.can_pan = can_pan;
        r
    }

    #[test]
    fn tap_and_double_tap() {
        let mut r = rec(false);
        r.down(1, 100.0, 100.0);
        assert_eq!(r.up(1, 102.0, 101.0, 1000), vec![Action::Tap(102.0, 101.0)]);
        r.down(1, 104.0, 100.0);
        assert_eq!(
            r.up(1, 104.0, 100.0, 1200),
            vec![Action::DoubleTap(104.0, 100.0)]
        );
        // A third tap starts over rather than being another double tap.
        r.down(1, 104.0, 100.0);
        assert_eq!(r.up(1, 104.0, 100.0, 1300), vec![Action::Tap(104.0, 100.0)]);
    }

    #[test]
    fn slow_second_tap_is_single() {
        let mut r = rec(false);
        r.down(1, 10.0, 10.0);
        r.up(1, 10.0, 10.0, 0);
        r.down(1, 10.0, 10.0);
        assert_eq!(r.up(1, 10.0, 10.0, 1000), vec![Action::Tap(10.0, 10.0)]);
    }

    #[test]
    fn swipe_left_goes_next_when_not_zoomed() {
        let mut r = rec(false);
        r.down(1, 300.0, 200.0);
        assert!(r.update(1, 250.0, 205.0).is_empty());
        assert_eq!(r.up(1, 200.0, 210.0, 0), vec![Action::Swipe(1)]);
    }

    #[test]
    fn swipe_right_goes_previous() {
        let mut r = rec(false);
        r.down(1, 100.0, 200.0);
        assert_eq!(r.up(1, 220.0, 200.0, 0), vec![Action::Swipe(-1)]);
    }

    #[test]
    fn vertical_or_short_drags_are_not_swipes() {
        let mut r = rec(false);
        r.down(1, 100.0, 100.0);
        assert!(r.up(1, 130.0, 220.0, 0).is_empty());
        r.down(1, 100.0, 100.0);
        assert!(r.up(1, 130.0, 100.0, 5000).is_empty());
    }

    #[test]
    fn drag_pans_when_zoomed_without_jump_and_no_swipe() {
        let mut r = rec(true);
        r.down(1, 100.0, 100.0);
        assert!(r.update(1, 104.0, 100.0).is_empty()); // inside slop
        assert_eq!(r.update(1, 120.0, 100.0), vec![Action::Pan(20.0, 0.0)]); // whole distance from start
        assert_eq!(r.update(1, 130.0, 105.0), vec![Action::Pan(10.0, 5.0)]);
        assert_eq!(r.up(1, 300.0, 105.0, 0), vec![Action::Pan(170.0, 0.0)]);
    }

    #[test]
    fn pinch_zooms_around_midpoint() {
        let mut r = rec(false);
        r.down(1, 100.0, 100.0);
        r.down(2, 200.0, 100.0);
        // Second finger moves out: distance 100 -> 200, midpoint 150 -> 200.
        let a = r.update(2, 300.0, 100.0);
        assert_eq!(
            a,
            vec![Action::Pinch {
                factor: 2.0,
                from: (150.0, 100.0),
                to: (200.0, 100.0)
            }]
        );
    }

    #[test]
    fn up_after_cancel_does_nothing() {
        let mut r = rec(false);
        r.down(1, 300.0, 200.0);
        r.update(1, 200.0, 200.0);
        r.cancel();
        assert!(r.up(1, 100.0, 200.0, 0).is_empty());
        // ...and a cancelled tap doesn't pair with the next one into a double tap.
        r.down(1, 10.0, 10.0);
        r.up(1, 10.0, 10.0, 0);
        r.cancel();
        r.down(1, 10.0, 10.0);
        assert_eq!(r.up(1, 10.0, 10.0, 100), vec![Action::Tap(10.0, 10.0)]);
    }

    #[test]
    fn after_pinch_remaining_finger_pans_and_never_taps_or_swipes() {
        let mut r = rec(false);
        r.down(1, 100.0, 100.0);
        r.down(2, 200.0, 100.0);
        r.update(2, 300.0, 100.0);
        assert!(r.up(2, 300.0, 100.0, 0).is_empty());
        assert_eq!(r.update(1, 90.0, 100.0), vec![Action::Pan(-10.0, 0.0)]);
        assert_eq!(r.up(1, 10.0, 100.0, 10), vec![Action::Pan(-80.0, 0.0)]);
    }

    #[test]
    fn slop_scales_with_dpi() {
        let mut r = Recognizer::new(2.0);
        r.can_pan = true;
        r.down(1, 0.0, 0.0);
        assert!(r.update(1, 12.0, 0.0).is_empty()); // 12px < 16px slop at 200%
        assert_eq!(r.update(1, 20.0, 0.0), vec![Action::Pan(20.0, 0.0)]);
    }
}
