/// Smallest world spacing between grid cells.
const BASE_SPACING: f64 = 10.0;
/// Grid cells never get closer than this on screen (logical pixels).
const MIN_SCREEN_SPACING: f64 = 16.0;

/// Which grid is shown at a given zoom.
///
/// Two levels are drawn: a coarse one at `2 * spacing` and a fine one at `spacing`
/// whose opacity fades in as it gets sparser. When zooming out past the threshold the
/// fine level has faded to zero and the coarse one becomes the new fine one, so the
/// transition is continuous.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLevel {
    /// World distance between fine grid cells.
    pub spacing: f64,
    /// Opacity of the fine level in `[0, 1)`.
    pub fine_alpha: f64,
}

impl GridLevel {
    pub fn for_zoom(zoom: f64) -> Self {
        let steps = (MIN_SCREEN_SPACING / (BASE_SPACING * zoom))
            .log2()
            .ceil()
            .max(0.0);
        let spacing = BASE_SPACING * steps.exp2();
        let screen = spacing * zoom;
        let fine_alpha = ((screen - MIN_SCREEN_SPACING) / MIN_SCREEN_SPACING).clamp(0.0, 1.0);
        Self {
            spacing,
            fine_alpha,
        }
    }

    /// Spacing of the finest grid that is noticeably visible, for snapping to it.
    pub fn snap_spacing(&self) -> f64 {
        if self.fine_alpha > 0.2 {
            self.spacing
        } else {
            2.0 * self.spacing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;

    #[test]
    fn screen_spacing_stays_in_band() {
        let mut zoom = Camera::MIN_ZOOM;
        while zoom <= Camera::MAX_ZOOM {
            let level = GridLevel::for_zoom(zoom);
            let screen = level.spacing * zoom;
            assert!(screen >= MIN_SCREEN_SPACING - 1e-9, "zoom {zoom}: {screen}");
            // At high zoom the base spacing is the floor, so cells may grow beyond the band.
            if BASE_SPACING * zoom < MIN_SCREEN_SPACING {
                assert!(
                    screen < 2.0 * MIN_SCREEN_SPACING + 1e-9,
                    "zoom {zoom}: {screen}"
                );
            }
            zoom *= 1.03;
        }
    }

    #[test]
    fn transition_is_continuous() {
        // Just above a threshold the fine level is almost transparent; just below, the
        // former coarse level is the new fine level and almost opaque.
        let threshold = MIN_SCREEN_SPACING / (BASE_SPACING * 4.0);
        let above = GridLevel::for_zoom(threshold * 1.0001);
        let below = GridLevel::for_zoom(threshold * 0.9999);
        assert!(above.fine_alpha < 0.01);
        assert!(below.fine_alpha > 0.99);
        assert!((below.spacing - 2.0 * above.spacing).abs() < 1e-9);
    }

    #[test]
    fn snaps_to_the_visible_level() {
        let faint = GridLevel {
            spacing: 10.0,
            fine_alpha: 0.1,
        };
        assert_eq!(faint.snap_spacing(), 20.0);
        let clear = GridLevel {
            fine_alpha: 0.8,
            ..faint
        };
        assert_eq!(clear.snap_spacing(), 10.0);
    }
}
