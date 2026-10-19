//! Books: PDF files on the board shown a page at a time, like an open book, and turned in
//! place (see `pb_core::Book`). Pages are rendered when first turned to, off the UI thread
//! on the desktop, and the next one ahead of time, so reading on is instant.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pb_core::{AssetId, Book, Command, Document, Image, ObjectId, ObjectKind};
use pb_geom::kurbo::Rect;
use pb_io::pdf::{Page, Reader};

/// A rendered page, already among the board's files.
#[derive(Clone, Copy)]
struct Rendered {
    asset: AssetId,
    width: u32,
    height: u32,
}

/// What turning a page needs.
pub enum Turn {
    /// The page is ready: apply this.
    Ready(Command),
    /// The page must be rendered first: render page `page` of `pdf` with `reader`, then
    /// hand the result to [`Books::rendered`].
    Render {
        reader: Arc<Reader>,
        pdf: AssetId,
        page: u32,
    },
    /// Nothing to do: not a book, already there, or already on its way.
    Nothing,
}

#[derive(Default)]
pub struct Books {
    /// Open PDFs by file.
    readers: HashMap<AssetId, Arc<Reader>>,
    /// Pages rendered so far, by file and page.
    pages: HashMap<(AssetId, u32), Rendered>,
    /// Pages being rendered in the background.
    rendering: HashSet<(AssetId, u32)>,
    /// Books waiting for their page to be rendered: the page each turns to.
    waiting: HashMap<ObjectId, u32>,
}

/// The book object `id` is, with its bounds.
pub fn book_of(doc: &Document, id: ObjectId) -> Option<(Book, Rect)> {
    let object = doc.get(id)?;
    match &object.data.kind {
        ObjectKind::Image(Image {
            book: Some(book), ..
        }) => Some((*book, object.data.bounds)),
        _ => None,
    }
}

impl Books {
    /// Forgets everything (another board was opened).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The page book `id` shows, or is turning to.
    pub fn page(&self, doc: &Document, id: ObjectId) -> Option<u32> {
        let (book, _) = book_of(doc, id)?;
        Some(self.waiting.get(&id).copied().unwrap_or(book.page))
    }

    /// Turns book `id` to `page` (clamped to its pages).
    pub fn turn(&mut self, doc: &Document, id: ObjectId, page: u32) -> Result<Turn, String> {
        let Some((book, bounds)) = book_of(doc, id) else {
            return Ok(Turn::Nothing);
        };
        let page = page.min(book.pages.saturating_sub(1));
        if page == book.page {
            self.waiting.remove(&id);
            return Ok(Turn::Nothing);
        }
        if let Some(&rendered) = self.pages.get(&(book.pdf, page)) {
            self.waiting.remove(&id);
            return Ok(Turn::Ready(turn_command(id, page, rendered, bounds)));
        }
        self.waiting.insert(id, page);
        self.render(doc, book.pdf, page)
    }

    /// Starts rendering page `page` of `pdf` unless it is rendered or on its way: the page
    /// after the one just turned to, so reading on does not wait.
    pub fn render(&mut self, doc: &Document, pdf: AssetId, page: u32) -> Result<Turn, String> {
        if self.pages.contains_key(&(pdf, page)) || !self.rendering.insert((pdf, page)) {
            return Ok(Turn::Nothing);
        }
        match self.reader(doc, pdf) {
            Ok(reader) => Ok(Turn::Render { reader, pdf, page }),
            Err(err) => {
                self.rendering.remove(&(pdf, page));
                self.waiting
                    .retain(|&id, _| book_of(doc, id).is_some_and(|(b, _)| b.pdf != pdf));
                Err(err)
            }
        }
    }

    /// Takes in a rendered page: adds it to the board's files and turns the books waiting
    /// for it. Returns the commands to apply, or the error if a book was waiting for it.
    pub fn rendered(
        &mut self,
        doc: &mut Document,
        pdf: AssetId,
        page: u32,
        result: Result<Page, String>,
    ) -> Result<Vec<Command>, String> {
        self.rendering.remove(&(pdf, page));
        let waiting: Vec<ObjectId> = self
            .waiting
            .iter()
            .filter(|&(&id, &p)| p == page && book_of(doc, id).is_some_and(|(b, _)| b.pdf == pdf))
            .map(|(&id, _)| id)
            .collect();
        let page_data = match result {
            Ok(page_data) => page_data,
            // A page rendered ahead of time that fails is no news until it is turned to.
            Err(_) if waiting.is_empty() => return Ok(Vec::new()),
            Err(err) => {
                for id in &waiting {
                    self.waiting.remove(id);
                }
                return Err(err);
            }
        };
        let rendered = Rendered {
            asset: doc.add_asset(page_data.png),
            width: page_data.width,
            height: page_data.height,
        };
        self.pages.insert((pdf, page), rendered);
        Ok(waiting
            .into_iter()
            .filter_map(|id| {
                self.waiting.remove(&id);
                let (_, bounds) = book_of(doc, id)?;
                Some(turn_command(id, page, rendered, bounds))
            })
            .collect())
    }

    /// The open PDF `pdf`, opened now if need be.
    fn reader(&mut self, doc: &Document, pdf: AssetId) -> Result<Arc<Reader>, String> {
        if let Some(reader) = self.readers.get(&pdf) {
            return Ok(reader.clone());
        }
        let bytes = doc
            .assets()
            .get(pdf)
            .ok_or_else(|| "the PDF file is missing from the board".to_owned())?;
        let reader = Arc::new(Reader::open(bytes)?);
        self.readers.insert(pdf, reader.clone());
        Ok(reader)
    }
}

/// Shows page `page` in book `id`: as wide as the book is, as tall as the page needs.
fn turn_command(id: ObjectId, page: u32, rendered: Rendered, bounds: Rect) -> Command {
    let width = bounds.width();
    let height = width * f64::from(rendered.height) / f64::from(rendered.width.max(1));
    Command::TurnPage {
        id,
        page,
        asset: rendered.asset,
        bounds: Rect::new(bounds.x0, bounds.y0, bounds.x0 + width, bounds.y0 + height),
    }
}

#[cfg(test)]
mod tests {
    use pb_core::{ObjectData, Style};

    use super::*;

    /// A board with a book of `pages` blank pages (a PDF made of pictures), on page 0.
    fn board(pages: usize) -> (Document, ObjectId) {
        let pixels = pb_io::image::Pixels {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        let jpeg = pb_io::image::encode_jpeg(&pixels, 90).unwrap();
        let pictures: Vec<_> = (0..pages)
            .map(|_| pb_io::pdf::PicturePage {
                jpeg: &jpeg,
                pixels: (2, 2),
                size: (100.0, 200.0),
            })
            .collect();
        let pdf = pb_io::pdf::write_pictures(&pictures);
        let mut doc = Document::new();
        let first = doc.add_asset(b"page 1".to_vec());
        let pdf = doc.add_asset(pdf);
        let data = ObjectData {
            kind: ObjectKind::Image(Image {
                book: Some(Book {
                    pdf,
                    page: 0,
                    pages: pages as u32,
                }),
                ..Image::new(first)
            }),
            bounds: Rect::new(0.0, 0.0, 50.0, 50.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        let applied = doc.apply(Command::Create(vec![data.into()])).unwrap();
        (doc, applied.created[0])
    }

    /// Renders what `turn` asks for, as the app does, and applies the result.
    fn finish(books: &mut Books, doc: &mut Document, turn: Turn) {
        match turn {
            Turn::Ready(command) => {
                doc.apply(command).unwrap();
            }
            Turn::Render { reader, pdf, page } => {
                let result = reader.render(page as usize);
                for command in books.rendered(doc, pdf, page, result).unwrap() {
                    doc.apply(command).unwrap();
                }
            }
            Turn::Nothing => {}
        }
    }

    #[test]
    fn turns_pages_in_place_and_reuses_rendered_ones() {
        let (mut doc, id) = board(3);
        let mut books = Books::default();
        let turn = books.turn(&doc, id, 1).unwrap();
        assert!(matches!(turn, Turn::Render { page: 1, .. }));
        // Asked again while rendering: nothing more to start.
        assert!(matches!(books.turn(&doc, id, 1).unwrap(), Turn::Nothing));
        assert_eq!(books.page(&doc, id), Some(1));
        finish(&mut books, &mut doc, turn);
        let (book, bounds) = book_of(&doc, id).unwrap();
        assert_eq!(book.page, 1);
        // Same width and corner; as tall as the page's shape.
        assert_eq!(bounds, Rect::new(0.0, 0.0, 50.0, 100.0));
        // Back and forth: page 1 is rendered already.
        let back = books.turn(&doc, id, 0).unwrap();
        finish(&mut books, &mut doc, back);
        assert!(matches!(books.turn(&doc, id, 1).unwrap(), Turn::Ready(_)));
        // Past the end stays on the last page.
        let end = books.turn(&doc, id, 99).unwrap();
        assert!(matches!(end, Turn::Render { page: 2, .. }));
    }

    #[test]
    fn a_failed_page_is_reported_only_when_turned_to() {
        let (mut doc, id) = board(2);
        let (book, _) = book_of(&doc, id).unwrap();
        let mut books = Books::default();
        // Rendered ahead of time, nobody waiting.
        let result = books.rendered(&mut doc, book.pdf, 1, Err("broken".into()));
        assert!(result.unwrap().is_empty());
        // A book that waited for it stays where it was.
        assert!(matches!(books.turn(&doc, id, 1), Ok(Turn::Render { .. })));
        let result = books.rendered(&mut doc, book.pdf, 1, Err("broken".into()));
        assert!(result.is_err());
        assert_eq!(books.page(&doc, id), Some(0));
    }
}
