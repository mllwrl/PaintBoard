//! What objects are drawn on: the GPU's scene, or a recording that becomes a vector file
//! (see [`crate::vector`]).

use vello::Scene;
use vello::kurbo::{Affine, Rect, Shape, Stroke};
use vello::peniko::{Color, Fill, ImageBrush, Mix};

/// The drawing operations objects are made of. Shapes are given in the space `transform`
/// maps to the output.
pub trait Canvas {
    fn fill(&mut self, transform: Affine, brush: Color, shape: &impl Shape);

    fn stroke(&mut self, style: &Stroke, transform: Affine, brush: Color, shape: &impl Shape);

    /// Starts a group drawn at `alpha` as a whole, cut to `clip`; ended by
    /// [`Canvas::pop_layer`].
    fn push_layer(&mut self, alpha: f32, transform: Affine, clip: &impl Shape);

    /// Starts a group cut to `clip`; ended by [`Canvas::pop_layer`].
    fn push_clip(&mut self, transform: Affine, clip: &impl Shape);

    fn pop_layer(&mut self);

    /// A soft shadow: `rect` blurred by `std_dev`.
    fn shadow(&mut self, transform: Affine, rect: Rect, brush: Color, std_dev: f64);

    /// Glyphs of `run`, positioned relative to `transform`'s origin.
    fn glyphs(&mut self, transform: Affine, run: &pb_text::GlyphRun, brush: Color);

    /// The picture of `image` placed by `image_transform` (from its pixels to the space
    /// of `transform`) and cut to `bounds`. `file` is the image file it was decoded from.
    fn image(
        &mut self,
        transform: Affine,
        bounds: Rect,
        image: &ImageBrush,
        image_transform: Affine,
        file: Option<&[u8]>,
    );
}

impl Canvas for Scene {
    fn fill(&mut self, transform: Affine, brush: Color, shape: &impl Shape) {
        Scene::fill(self, Fill::NonZero, transform, brush, None, shape);
    }

    fn stroke(&mut self, style: &Stroke, transform: Affine, brush: Color, shape: &impl Shape) {
        Scene::stroke(self, style, transform, brush, None, shape);
    }

    fn push_layer(&mut self, alpha: f32, transform: Affine, clip: &impl Shape) {
        Scene::push_layer(self, Fill::NonZero, Mix::Normal, alpha, transform, clip);
    }

    fn push_clip(&mut self, transform: Affine, clip: &impl Shape) {
        Scene::push_clip_layer(self, Fill::NonZero, transform, clip);
    }

    fn pop_layer(&mut self) {
        Scene::pop_layer(self);
    }

    fn shadow(&mut self, transform: Affine, rect: Rect, brush: Color, std_dev: f64) {
        self.draw_blurred_rounded_rect(transform, rect, brush, 0.0, std_dev);
    }

    fn glyphs(&mut self, transform: Affine, run: &pb_text::GlyphRun, brush: Color) {
        let glyphs = run.glyphs.iter().map(|g| vello::Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        });
        self.draw_glyphs(&run.font)
            .font_size(run.size)
            .transform(transform)
            .brush(brush)
            .draw(Fill::NonZero, glyphs);
    }

    fn image(
        &mut self,
        transform: Affine,
        bounds: Rect,
        image: &ImageBrush,
        image_transform: Affine,
        _file: Option<&[u8]>,
    ) {
        Scene::fill(
            self,
            Fill::NonZero,
            transform,
            image,
            Some(image_transform),
            &bounds,
        );
    }
}
