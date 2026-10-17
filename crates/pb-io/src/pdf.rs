//! PDF files: their pages rendered to pictures, to place on the board.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

/// A rendered page: PNG bytes and the size in pixels.
pub struct Page {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Pages are rendered at this many pixels per PDF point (1/72 inch): an A4 page becomes
/// about 1240 pixels wide, sharp at 100% zoom and a little beyond.
const SCALE: f32 = 2.0;
/// Pages are never rendered larger than this on their longer side, in pixels.
const MAX_SIDE: f32 = 4096.0;

/// How many pages `bytes` has, or why it cannot be read.
pub fn page_count(bytes: &[u8]) -> Result<usize, String> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("not a readable PDF ({e:?})"))?;
    Ok(pdf.pages().len())
}

/// Renders the first `max_pages` pages of the PDF in `bytes`, white behind them.
pub fn render_pages(bytes: &[u8], max_pages: usize) -> Result<Vec<Page>, String> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("not a readable PDF ({e:?})"))?;
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let mut pages = Vec::new();
    for page in pdf.pages().iter().take(max_pages) {
        let (w, h) = page.render_dimensions();
        let scale = SCALE.min(MAX_SIDE / w.max(h).max(1.0));
        let pixmap = render(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        if width == 0 || height == 0 {
            continue;
        }
        let png = pixmap.into_png().map_err(|e| e.to_string())?;
        pages.push(Page { png, width, height });
    }
    if pages.is_empty() {
        return Err("the PDF has no pages to show".into());
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF with a filled square, written by hand.
    fn tiny_pdf() -> Vec<u8> {
        let content = b"0 0 1 rg 20 20 60 60 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                std::str::from_utf8(content).unwrap()
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    #[test]
    fn pages_become_pictures() {
        let pdf = tiny_pdf();
        assert_eq!(page_count(&pdf), Ok(1));
        let pages = render_pages(&pdf, 10).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].width, pages[0].height), (200, 200));
        let decoded = image_dims(&pages[0].png);
        assert_eq!(decoded, (200, 200));
        assert!(render_pages(b"not a pdf", 10).is_err());
    }

    fn image_dims(png: &[u8]) -> (u32, u32) {
        crate::image::dimensions(png).unwrap()
    }
}
