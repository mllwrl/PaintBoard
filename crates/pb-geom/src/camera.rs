use kurbo::{Affine, Point, Rect, Size, Vec2};

/// Maps the infinite world plane onto the screen.
///
/// `screen = (world - origin) * zoom`, where `origin` is the world point shown at the
/// top-left corner of the viewport. Coordinates are `f64` so precision holds far from zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    origin: Point,
    zoom: f64,
}

impl Camera {
    pub const MIN_ZOOM: f64 = 0.02;
    pub const MAX_ZOOM: f64 = 32.0;

    pub fn new(origin: Point, zoom: f64) -> Self {
        Self {
            origin,
            zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM),
        }
    }

    pub fn origin(&self) -> Point {
        self.origin
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn world_to_screen(&self, p: Point) -> Point {
        ((p - self.origin) * self.zoom).to_point()
    }

    pub fn screen_to_world(&self, p: Point) -> Point {
        self.origin + p.to_vec2() / self.zoom
    }

    /// World-to-screen transform, for rendering world-space geometry.
    pub fn transform(&self) -> Affine {
        Affine::scale(self.zoom) * Affine::translate(-self.origin.to_vec2())
    }

    /// The part of the world visible in a viewport of the given size.
    pub fn visible_world_rect(&self, viewport: Size) -> Rect {
        Rect::from_points(
            self.origin,
            self.screen_to_world(viewport.to_vec2().to_point()),
        )
    }

    /// Moves the view so that content follows a pointer that moved by `screen_delta`.
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.origin -= screen_delta / self.zoom;
    }

    /// Multiplies the zoom by `factor`, keeping the world point under `screen_anchor` fixed.
    pub fn zoom_at(&mut self, screen_anchor: Point, factor: f64) {
        self.set_zoom_at(screen_anchor, self.zoom * factor);
    }

    /// Sets the zoom, keeping the world point under `screen_anchor` fixed.
    pub fn set_zoom_at(&mut self, screen_anchor: Point, zoom: f64) {
        let anchor_world = self.screen_to_world(screen_anchor);
        self.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.origin = anchor_world - screen_anchor.to_vec2() / self.zoom;
    }

    /// Centers the view on `world_center` at the given zoom.
    pub fn look_at(&mut self, world_center: Point, zoom: f64, viewport: Size) {
        self.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.origin = world_center - viewport.to_vec2() / (2.0 * self.zoom);
    }

    /// Shows all of `area` (world), as large as fits with `margin` logical pixels around it.
    pub fn fit(&mut self, area: Rect, viewport: Size, margin: f64) {
        let room = Size::new(
            (viewport.width - 2.0 * margin).max(1.0),
            (viewport.height - 2.0 * margin).max(1.0),
        );
        let zoom = (room.width / area.width().max(f64::EPSILON))
            .min(room.height / area.height().max(f64::EPSILON));
        self.look_at(area.center(), zoom, viewport);
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(Point::ZERO, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shows_the_whole_area_centered() {
        let mut camera = Camera::default();
        let viewport = Size::new(1000.0, 600.0);
        camera.fit(Rect::new(100.0, 100.0, 500.0, 300.0), viewport, 100.0);
        // 800 x 400 of room for 400 x 200: limited by both, twice the size.
        assert_eq!(camera.zoom(), 2.0);
        let center = camera.world_to_screen(Point::new(300.0, 200.0));
        assert_eq!(center, Point::new(500.0, 300.0));
    }

    fn assert_close(a: Point, b: Point) {
        assert!((a - b).hypot() < 1e-9, "{a:?} != {b:?}");
    }

    #[test]
    fn screen_world_roundtrip() {
        let cam = Camera::new(Point::new(-120.5, 40.0), 2.5);
        let p = Point::new(333.0, -17.25);
        assert_close(cam.screen_to_world(cam.world_to_screen(p)), p);
        assert_close(cam.transform() * p, cam.world_to_screen(p));
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut cam = Camera::new(Point::new(10.0, 20.0), 1.0);
        let anchor = Point::new(400.0, 300.0);
        let before = cam.screen_to_world(anchor);
        cam.zoom_at(anchor, 3.7);
        assert_close(cam.screen_to_world(anchor), before);
        cam.zoom_at(anchor, 0.1);
        assert_close(cam.screen_to_world(anchor), before);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut cam = Camera::default();
        cam.zoom_at(Point::ZERO, 1e9);
        assert_eq!(cam.zoom(), Camera::MAX_ZOOM);
        cam.zoom_at(Point::ZERO, 1e-9);
        assert_eq!(cam.zoom(), Camera::MIN_ZOOM);
    }

    #[test]
    fn pan_follows_pointer() {
        let mut cam = Camera::new(Point::ZERO, 2.0);
        let world = Point::new(50.0, 50.0);
        let before = cam.world_to_screen(world);
        cam.pan(Vec2::new(30.0, -10.0));
        assert_close(cam.world_to_screen(world), before + Vec2::new(30.0, -10.0));
    }

    #[test]
    fn look_at_centers() {
        let mut cam = Camera::default();
        let viewport = Size::new(800.0, 600.0);
        cam.look_at(Point::new(1000.0, -500.0), 4.0, viewport);
        assert_close(
            cam.screen_to_world(Point::new(400.0, 300.0)),
            Point::new(1000.0, -500.0),
        );
        assert_close(
            cam.visible_world_rect(viewport).center(),
            Point::new(1000.0, -500.0),
        );
    }
}
