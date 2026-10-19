//! PDF files: their pages rendered to pictures, to place on the board, and PDF files
//! made of pictures of the board.

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
    Ok(Reader::open(bytes)?.page_count())
}

/// Renders the first `max_pages` pages of the PDF in `bytes`, white behind them.
pub fn render_pages(bytes: &[u8], max_pages: usize) -> Result<Vec<Page>, String> {
    let reader = Reader::open(bytes)?;
    let pages: Vec<Page> = (0..reader.page_count().min(max_pages))
        .filter_map(|i| reader.render(i).ok())
        .collect();
    if pages.is_empty() {
        return Err("the PDF has no pages to show".into());
    }
    Ok(pages)
}

/// An open PDF whose pages are rendered one at a time, as a book on the board turns them.
pub struct Reader {
    pdf: Pdf,
}

impl Reader {
    /// Reads the PDF in `bytes`, or says why it cannot.
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("not a readable PDF ({e:?})"))?;
        Ok(Self { pdf })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Page `index` (from 0), white behind it.
    pub fn render(&self, index: usize) -> Result<Page, String> {
        let page = self
            .pdf
            .pages()
            .get(index)
            .ok_or_else(|| format!("the PDF has no page {}", index + 1))?;
        let (w, h) = page.render_dimensions();
        let scale = SCALE.min(MAX_SIDE / w.max(h).max(1.0));
        let pixmap = render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        if width == 0 || height == 0 {
            return Err(format!("page {} is empty", index + 1));
        }
        let png = pixmap.into_png().map_err(|e| e.to_string())?;
        Ok(Page { png, width, height })
    }
}

/// A page of a PDF being written: one JPEG picture covering it.
pub struct PicturePage<'a> {
    pub jpeg: &'a [u8],
    /// The picture's size in pixels.
    pub pixels: (u32, u32),
    /// The page's size in points (1/72 inch).
    pub size: (f64, f64),
}

/// The largest page side PDF readers accept, in points (200 inches).
pub const MAX_PAGE_SIDE: f64 = 14400.0;

/// A PDF file with one page per picture, each picture filling its page.
pub fn write_pictures(pages: &[PicturePage<'_>]) -> Vec<u8> {
    // Objects: 1 catalog, 2 page tree, then a page, its contents and its picture per page.
    let page_id = |i: usize| 3 + i * 3;
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", offsets.len()).bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", page_id(i)))
        .collect();
    let tree = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
    object(&mut out, tree.as_bytes());
    for (i, page) in pages.iter().enumerate() {
        let id = page_id(i);
        let (w, h) = page.size;
        let fit = (MAX_PAGE_SIDE / w.max(h)).min(1.0);
        let (w, h) = (w * fit, h * fit);
        let body = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] \
             /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
            id + 2,
            id + 1
        );
        object(&mut out, body.as_bytes());
        let content = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im0 Do Q");
        let body = format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        );
        object(&mut out, body.as_bytes());
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            page.pixels.0,
            page.pixels.1,
            page.jpeg.len()
        )
        .into_bytes();
        body.extend_from_slice(page.jpeg);
        body.extend_from_slice(b"\nendstream");
        object(&mut out, &body);
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).bytes());
    for offset in &offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_pictures_read_back_as_pages() {
        let pixels = crate::image::Pixels {
            width: 40,
            height: 20,
            rgba: [200, 30, 30, 255].repeat(40 * 20),
        };
        let jpeg = crate::image::encode_jpeg(&pixels, 90).unwrap();
        let page = |size| PicturePage {
            jpeg: &jpeg,
            pixels: (40, 20),
            size,
        };
        let bytes = write_pictures(&[page((300.0, 150.0)), page((100.0, 50.0))]);
        assert_eq!(page_count(&bytes).unwrap(), 2);
        let pages = render_pages(&bytes, 2).unwrap();
        assert_eq!((pages[0].width, pages[0].height), (600, 300));
        let first = crate::image::decode(&pages[0].png, 4096).unwrap();
        let middle = ((150 * first.width + 300) * 4) as usize;
        let [r, g, b, _] = first.rgba[middle..middle + 4] else {
            unreachable!()
        };
        assert!(r > 150 && g < 80 && b < 80, "{r} {g} {b}");
    }

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

    #[test]
    fn reader_renders_any_page() {
        let pixels = crate::image::Pixels {
            width: 2,
            height: 2,
            rgba: [0; 16].to_vec(),
        };
        let jpeg = crate::image::encode_jpeg(&pixels, 90).unwrap();
        let page = |size| PicturePage {
            jpeg: &jpeg,
            pixels: (2, 2),
            size,
        };
        let bytes = write_pictures(&[page((100.0, 50.0)), page((50.0, 100.0))]);
        let reader = Reader::open(&bytes).unwrap();
        assert_eq!(reader.page_count(), 2);
        let second = reader.render(1).unwrap();
        assert_eq!((second.width, second.height), (100, 200));
        assert!(reader.render(2).is_err());
        assert!(Reader::open(b"not a pdf").is_err());
    }

    fn image_dims(png: &[u8]) -> (u32, u32) {
        crate::image::dimensions(png).unwrap()
    }
}
