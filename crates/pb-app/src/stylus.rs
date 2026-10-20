//! What pen and finger samples mean on the canvas: drawing, erasing, the right-click menu,
//! moving the board or nothing at all (a palm resting on the screen while writing).
//!
//! Kept free of windows and events so the rules can be tested with made-up samples.
//!
//! - A pen draws with the picked tool; turned over (eraser end) it erases instead, and so it
//!   does with its side (barrel) button held. A tap with the side button held, without
//!   moving, is a right click.
//! - While a pen is near the screen, and for [`PEN_GRACE`] after it was last seen, fingers do
//!   nothing: they are the hand holding it.
//! - With "pen draws, fingers move the board" on, one finger pans and two pinch-zoom; else the
//!   first finger acts as the mouse, as on a touch screen without a pen.

use pb_geom::kurbo::{Point, Vec2};

/// How long after a pen was last seen fingers are still taken for the hand holding it, in
/// seconds.
pub const PEN_GRACE: f64 = 0.5;
/// How far (in pixels) a pen may move with its side button held and still be a tap.
pub const TAP_SLOP: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pen,
    Touch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Touches the screen.
    Down,
    /// Moves, touching or (a pen) hovering.
    Move,
    /// Leaves the screen.
    Up,
    /// The system took the pointer away.
    Cancel,
}

/// One pointer report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub id: u64,
    pub kind: Kind,
    /// The pen's eraser end is toward the screen, or its eraser button is pressed.
    pub eraser: bool,
    /// The pen's side button is pressed.
    pub barrel: bool,
    pub phase: Phase,
    /// In window pixels.
    pub pos: Point,
    /// In seconds, from any fixed moment.
    pub time: f64,
}

/// What to do for a sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// The pointer moves without pressing (a pen hovering); `erase` says whether it would
    /// erase.
    Hover { pos: Point, erase: bool },
    /// Like pressing the left button at `pos`, erasing whatever tool is picked when `erase`.
    Press { pos: Point, erase: bool },
    /// The pressed pointer moves.
    Drag(Point),
    /// Like releasing the left button.
    Release(Point),
    /// The press so far is dropped, nothing done: a finger turned out to be the palm.
    Cancel,
    /// Like a right click at a point.
    ContextMenu(Point),
    /// Move the board by window pixels.
    Pan(Vec2),
    /// Move the board by `pan`, then zoom by `factor` around `center`.
    Zoom {
        center: Point,
        factor: f64,
        pan: Vec2,
    },
}

/// The pointer acting as the mouse's left button.
#[derive(Clone, Copy, Debug)]
struct Driver {
    id: u64,
    kind: Kind,
    erase: bool,
    /// False while a tap with the side button may still be a right click.
    pressed: bool,
    start: Point,
}

#[derive(Default)]
pub struct Stylus {
    /// When a pen was last reported.
    pen_at: Option<f64>,
    driver: Option<Driver>,
    /// Fingers moving the board, and where each was last.
    fingers: Vec<(u64, Point)>,
    /// Fingers taken for the palm, ignored until they leave the screen.
    ignored: Vec<u64>,
}

impl Stylus {
    /// What sample `s` means; `None` when it is to be ignored altogether (also by the UI):
    /// a palm. `fingers_navigate`: fingers move the board rather than act as the mouse.
    pub fn feed(&mut self, s: Sample, fingers_navigate: bool) -> Option<Vec<Out>> {
        match s.kind {
            Kind::Pen => Some(self.pen(s)),
            Kind::Touch => self.touch(s, fingers_navigate),
        }
    }

    /// Whether a pen is near the screen at `time`, or was a moment ago.
    pub fn pen_near(&self, time: f64) -> bool {
        self.driver.is_some_and(|d| d.kind == Kind::Pen)
            || self.pen_at.is_some_and(|at| time - at < PEN_GRACE)
    }

    fn pen(&mut self, s: Sample) -> Vec<Out> {
        self.pen_at = Some(s.time);
        let mut out = Vec::new();
        // A finger acting as the mouse was the hand holding the pen; fingers moving the
        // board stop.
        if let Some(d) = self.driver.filter(|d| d.kind == Kind::Touch) {
            self.ignored.push(d.id);
            self.driver = None;
            out.push(Out::Cancel);
        }
        self.ignored
            .extend(self.fingers.drain(..).map(|(id, _)| id));
        let erase = s.eraser || s.barrel;
        let mine = self.driver.filter(|d| d.id == s.id);
        match s.phase {
            Phase::Down => {
                // A pen still down touches again: its lifting was missed.
                if let Some(d) = self.driver.take()
                    && d.pressed
                {
                    out.push(Out::Release(s.pos));
                }
                // With only the side button held, wait: a tap is a right click.
                let pressed = !(s.barrel && !s.eraser);
                self.driver = Some(Driver {
                    id: s.id,
                    kind: Kind::Pen,
                    erase,
                    pressed,
                    start: s.pos,
                });
                if pressed {
                    out.push(Out::Press { pos: s.pos, erase });
                }
            }
            Phase::Move => match mine {
                None if self.driver.is_none() => out.push(Out::Hover { pos: s.pos, erase }),
                None => {}
                Some(d) if !d.pressed => {
                    // The side button let go before moving: an ordinary stroke.
                    let stroke = !s.barrel;
                    if stroke || (s.pos - d.start).hypot() > TAP_SLOP {
                        let erase = if stroke { s.eraser } else { true };
                        self.driver = Some(Driver {
                            pressed: true,
                            erase,
                            ..d
                        });
                        out.push(Out::Press {
                            pos: d.start,
                            erase,
                        });
                        out.push(Out::Drag(s.pos));
                    }
                }
                Some(d) if d.erase != erase => {
                    // Turned over or the side button changed mid-stroke: end it, go on with
                    // the other one.
                    self.driver = Some(Driver { erase, ..d });
                    out.push(Out::Release(s.pos));
                    out.push(Out::Press { pos: s.pos, erase });
                }
                Some(_) => out.push(Out::Drag(s.pos)),
            },
            Phase::Up | Phase::Cancel => {
                if let Some(d) = mine {
                    self.driver = None;
                    if d.pressed {
                        out.push(Out::Release(s.pos));
                    } else if s.phase == Phase::Up {
                        out.push(Out::ContextMenu(d.start));
                    }
                }
            }
        }
        out
    }

    fn touch(&mut self, s: Sample, fingers_navigate: bool) -> Option<Vec<Out>> {
        let leaves = matches!(s.phase, Phase::Up | Phase::Cancel);
        if let Some(i) = self.ignored.iter().position(|&id| id == s.id) {
            if leaves || s.phase == Phase::Down {
                self.ignored.swap_remove(i);
            }
            // Touching again under the same id: its leaving was missed, it is a new touch.
            if s.phase != Phase::Down {
                return None;
            }
        }
        if self.pen_near(s.time) {
            if s.phase == Phase::Down {
                self.ignored.push(s.id);
            }
            return None;
        }
        if self.driver.is_some_and(|d| d.id == s.id) {
            // A finger acting as the mouse goes on doing so even if the setting changed.
            return Some(match s.phase {
                Phase::Down | Phase::Move => vec![Out::Drag(s.pos)],
                Phase::Up | Phase::Cancel => {
                    self.driver = None;
                    vec![Out::Release(s.pos)]
                }
            });
        }
        if fingers_navigate || !self.fingers.is_empty() {
            return Some(self.navigate(s));
        }
        Some(match s.phase {
            Phase::Down if self.driver.is_none() => {
                self.driver = Some(Driver {
                    id: s.id,
                    kind: Kind::Touch,
                    erase: false,
                    pressed: true,
                    start: s.pos,
                });
                vec![Out::Press {
                    pos: s.pos,
                    erase: false,
                }]
            }
            _ => Vec::new(),
        })
    }

    /// One finger pans, two pinch-zoom around their middle; more are left out.
    fn navigate(&mut self, s: Sample) -> Vec<Out> {
        let at = self.fingers.iter().position(|&(id, _)| id == s.id);
        match (s.phase, at) {
            (Phase::Down, None) if self.fingers.len() < 2 => {
                self.fingers.push((s.id, s.pos));
                Vec::new()
            }
            (Phase::Down | Phase::Move, Some(i)) => {
                let before = self.fingers.clone();
                self.fingers[i].1 = s.pos;
                match (before.as_slice(), self.fingers.as_slice()) {
                    ([(_, a)], [(_, b)]) => vec![Out::Pan(*b - *a)],
                    ([(_, a0), (_, a1)], [(_, b0), (_, b1)]) => {
                        let (old_mid, new_mid) = (a0.midpoint(*a1), b0.midpoint(*b1));
                        let (old_len, new_len) = ((*a1 - *a0).hypot(), (*b1 - *b0).hypot());
                        let factor = if old_len > 1.0 && new_len > 1.0 {
                            new_len / old_len
                        } else {
                            1.0
                        };
                        vec![Out::Zoom {
                            center: new_mid,
                            factor,
                            pan: new_mid - old_mid,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            (Phase::Up | Phase::Cancel, Some(i)) => {
                self.fingers.remove(i);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pen(phase: Phase, x: f64, time: f64) -> Sample {
        Sample {
            id: 1,
            kind: Kind::Pen,
            eraser: false,
            barrel: false,
            phase,
            pos: Point::new(x, 0.0),
            time,
        }
    }

    fn finger(id: u64, phase: Phase, x: f64, y: f64, time: f64) -> Sample {
        Sample {
            id,
            kind: Kind::Touch,
            eraser: false,
            barrel: false,
            phase,
            pos: Point::new(x, y),
            time,
        }
    }

    fn feed(s: &mut Stylus, sample: Sample) -> Vec<Out> {
        s.feed(sample, false).expect("not ignored")
    }

    #[test]
    fn pen_draws_and_hovers() {
        let mut s = Stylus::default();
        assert_eq!(
            feed(&mut s, pen(Phase::Move, 5.0, 0.0)),
            [Out::Hover {
                pos: Point::new(5.0, 0.0),
                erase: false
            }]
        );
        assert_eq!(
            feed(&mut s, pen(Phase::Down, 5.0, 0.1)),
            [Out::Press {
                pos: Point::new(5.0, 0.0),
                erase: false
            }]
        );
        assert_eq!(
            feed(&mut s, pen(Phase::Move, 9.0, 0.2)),
            [Out::Drag(Point::new(9.0, 0.0))]
        );
        assert_eq!(
            feed(&mut s, pen(Phase::Up, 9.0, 0.3)),
            [Out::Release(Point::new(9.0, 0.0))]
        );
    }

    #[test]
    fn eraser_end_erases() {
        let mut s = Stylus::default();
        let flipped = |phase, x, t| Sample {
            eraser: true,
            ..pen(phase, x, t)
        };
        assert_eq!(
            feed(&mut s, flipped(Phase::Move, 0.0, 0.0)),
            [Out::Hover {
                pos: Point::ZERO,
                erase: true
            }]
        );
        assert_eq!(
            feed(&mut s, flipped(Phase::Down, 0.0, 0.1)),
            [Out::Press {
                pos: Point::ZERO,
                erase: true
            }]
        );
        assert_eq!(
            feed(&mut s, flipped(Phase::Move, 4.0, 0.2)),
            [Out::Drag(Point::new(4.0, 0.0))]
        );
        feed(&mut s, flipped(Phase::Up, 4.0, 0.3));
        // Turned back: draws again.
        assert_eq!(
            feed(&mut s, pen(Phase::Down, 4.0, 1.0)),
            [Out::Press {
                pos: Point::new(4.0, 0.0),
                erase: false
            }]
        );
    }

    #[test]
    fn side_button_stroke_erases_and_tap_is_right_click() {
        let mut s = Stylus::default();
        let barrel = |phase, x, t| Sample {
            barrel: true,
            ..pen(phase, x, t)
        };
        // A tap: nothing until it lifts, then the menu.
        assert!(feed(&mut s, barrel(Phase::Down, 0.0, 0.0)).is_empty());
        assert!(feed(&mut s, barrel(Phase::Move, 3.0, 0.1)).is_empty());
        assert_eq!(
            feed(&mut s, barrel(Phase::Up, 3.0, 0.2)),
            [Out::ContextMenu(Point::ZERO)]
        );
        // A stroke: erases from where it started.
        assert!(feed(&mut s, barrel(Phase::Down, 0.0, 1.0)).is_empty());
        assert_eq!(
            feed(&mut s, barrel(Phase::Move, 30.0, 1.1)),
            [
                Out::Press {
                    pos: Point::ZERO,
                    erase: true
                },
                Out::Drag(Point::new(30.0, 0.0))
            ]
        );
        assert_eq!(
            feed(&mut s, barrel(Phase::Up, 30.0, 1.2)),
            [Out::Release(Point::new(30.0, 0.0))]
        );
        // Pressed mid-stroke: the ink so far stays, erasing goes on.
        feed(&mut s, pen(Phase::Down, 0.0, 2.0));
        assert_eq!(
            feed(&mut s, barrel(Phase::Move, 5.0, 2.1)),
            [
                Out::Release(Point::new(5.0, 0.0)),
                Out::Press {
                    pos: Point::new(5.0, 0.0),
                    erase: true
                }
            ]
        );
    }

    #[test]
    fn palm_is_ignored_near_the_pen() {
        let mut s = Stylus::default();
        feed(&mut s, pen(Phase::Move, 0.0, 0.0));
        // Hovering pen: fingers are ignored, and stay so after it leaves.
        assert_eq!(s.feed(finger(7, Phase::Down, 1.0, 1.0, 0.1), false), None);
        assert_eq!(s.feed(finger(7, Phase::Move, 2.0, 1.0, 2.0), false), None);
        assert_eq!(s.feed(finger(7, Phase::Up, 2.0, 1.0, 2.1), false), None);
        // While drawing.
        feed(&mut s, pen(Phase::Down, 0.0, 3.0));
        assert_eq!(s.feed(finger(8, Phase::Down, 1.0, 1.0, 3.1), true), None);
        feed(&mut s, pen(Phase::Up, 0.0, 3.2));
        // Just after the pen left.
        assert_eq!(s.feed(finger(9, Phase::Down, 1.0, 1.0, 3.6), false), None);
        // Long after: a finger acts as the mouse again.
        assert_eq!(
            s.feed(finger(10, Phase::Down, 1.0, 1.0, 4.0), false),
            Some(vec![Out::Press {
                pos: Point::new(1.0, 1.0),
                erase: false
            }])
        );
    }

    #[test]
    fn palm_down_before_the_pen_is_dropped() {
        let mut s = Stylus::default();
        feed(&mut s, finger(7, Phase::Down, 1.0, 1.0, 0.0));
        assert_eq!(
            feed(&mut s, pen(Phase::Move, 0.0, 0.1)),
            [
                Out::Cancel,
                Out::Hover {
                    pos: Point::ZERO,
                    erase: false
                }
            ]
        );
        assert_eq!(s.feed(finger(7, Phase::Up, 1.0, 1.0, 5.0), false), None);
    }

    #[test]
    fn an_ignored_finger_whose_leaving_was_missed_counts_again() {
        let mut s = Stylus::default();
        feed(&mut s, pen(Phase::Move, 0.0, 0.0));
        assert_eq!(s.feed(finger(7, Phase::Down, 1.0, 1.0, 0.1), false), None);
        assert!(
            s.feed(finger(7, Phase::Down, 1.0, 1.0, 5.0), false)
                .is_some()
        );
    }

    #[test]
    fn plain_touch_screen_is_a_mouse() {
        let mut s = Stylus::default();
        assert_eq!(
            feed(&mut s, finger(3, Phase::Down, 1.0, 1.0, 0.0)),
            [Out::Press {
                pos: Point::new(1.0, 1.0),
                erase: false
            }]
        );
        // A second finger does nothing.
        assert!(feed(&mut s, finger(4, Phase::Down, 5.0, 5.0, 0.1)).is_empty());
        assert!(feed(&mut s, finger(4, Phase::Move, 6.0, 5.0, 0.1)).is_empty());
        assert_eq!(
            feed(&mut s, finger(3, Phase::Move, 2.0, 1.0, 0.2)),
            [Out::Drag(Point::new(2.0, 1.0))]
        );
        assert_eq!(
            feed(&mut s, finger(3, Phase::Up, 2.0, 1.0, 0.3)),
            [Out::Release(Point::new(2.0, 1.0))]
        );
    }

    #[test]
    fn fingers_pan_and_pinch() {
        let mut s = Stylus::default();
        let mut nav = |sample| s.feed(sample, true).expect("not ignored");
        assert!(nav(finger(3, Phase::Down, 0.0, 0.0, 0.0)).is_empty());
        assert_eq!(
            nav(finger(3, Phase::Move, 5.0, 2.0, 0.1)),
            [Out::Pan(Vec2::new(5.0, 2.0))]
        );
        assert!(nav(finger(4, Phase::Down, 15.0, 2.0, 0.2)).is_empty());
        // The second finger moves away: twice as far apart, the middle moves by 5.
        assert_eq!(
            nav(finger(4, Phase::Move, 25.0, 2.0, 0.3)),
            [Out::Zoom {
                center: Point::new(15.0, 2.0),
                factor: 2.0,
                pan: Vec2::new(5.0, 0.0)
            }]
        );
        // A third finger is left out.
        assert!(nav(finger(5, Phase::Down, 50.0, 50.0, 0.4)).is_empty());
        assert!(nav(finger(5, Phase::Move, 60.0, 50.0, 0.4)).is_empty());
        // One lifts: the other pans again, from where it is.
        assert!(nav(finger(3, Phase::Up, 5.0, 2.0, 0.5)).is_empty());
        assert_eq!(
            nav(finger(4, Phase::Move, 20.0, 2.0, 0.6)),
            [Out::Pan(Vec2::new(-5.0, 0.0))]
        );
        // Fingers never press.
        assert!(
            !nav(finger(4, Phase::Up, 20.0, 2.0, 0.7))
                .iter()
                .any(|o| matches!(o, Out::Press { .. } | Out::Release(_)))
        );
    }
}
