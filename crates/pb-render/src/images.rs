//! Decoded board images, ready for the GPU.
//!
//! Decoding a photo takes a good part of a second, so on the desktop it is done on worker
//! threads: a picture not decoded yet is drawn as a plain placeholder and the app is woken
//! when it is ready. The web has no threads and decodes one picture a frame instead.
//! Exports and printing switch the cache to [`ImageCache::set_blocking`], which decodes
//! right away.

use std::collections::HashMap;

use pb_core::{AssetId, Assets};
use vello::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

/// Images are decoded at most this large on their longer side; vello keeps images in an
/// atlas, and bigger ones would not fit.
const MAX_SIDE: u32 = 4096;

/// Smaller copies of an image stop halving at this longer side.
const MIN_SIDE: u32 = 64;

/// When decoded pictures take more memory than the budget, copies bigger than this are
/// dropped first, so a zoomed-out board full of photos stays cheap.
const KEEP_SIDE: u32 = 1024;

/// Memory for decoded pixels; past it, the biggest copies of pictures not drawn lately go.
#[cfg(not(target_arch = "wasm32"))]
const BUDGET: usize = 1 << 30;
#[cfg(target_arch = "wasm32")]
const BUDGET: usize = 512 << 20;

/// What there is to draw for a picture.
pub enum Picture<'a> {
    Ready(&'a ImageBrush),
    /// Being decoded: draw a placeholder; the app is woken (or [`ImageCache::busy`] says
    /// so) when it is ready.
    Loading,
    /// The file is missing or cannot be decoded.
    Broken,
}

/// Decoded images by asset, decoded on first use.
///
/// vello copies every image drawn in a frame into one atlas of at most 8192×8192 pixels,
/// and an image that does not fit is silently not drawn. So an image is drawn from a copy
/// just big enough for its size on screen (halved from the decoded one), which keeps a
/// screen full of photos and PDF pages well inside the atlas.
pub struct ImageCache {
    entries: HashMap<AssetId, Entry>,
    mode: Mode,
    /// Decodes right away whatever the mode (exports).
    blocking: bool,
    /// Counts [`ImageCache::begin_frame`]s: when each picture was last drawn.
    frame: u64,
    budget: usize,
    /// See [`KEEP_SIDE`].
    keep_side: u32,
}

enum Mode {
    /// Decodes when asked (tests, exports with their own cache).
    Blocking,
    /// One decode a frame, the rest wait for later frames (the web).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Progressive { left: u32, deferred: bool },
    /// Worker threads decode.
    #[cfg(not(target_arch = "wasm32"))]
    Background(pool::Pool),
}

/// A picture's decoded copies.
#[derive(Default)]
struct Entry {
    /// The full decoded image, then each copy half as large as the one before, down to
    /// [`MIN_SIDE`]; the biggest may have been dropped to save memory. Empty until decoded.
    levels: Vec<ImageBrush>,
    /// The longer side of the full decoded image; 0 until decoded.
    full: u32,
    failed: bool,
    /// The decode on its way, if any.
    job: Option<u64>,
    /// The frame it was last drawn in, and the longest side of the copy drawn then.
    used: u64,
    used_side: u32,
}

impl Default for ImageCache {
    /// A cache that decodes right away.
    fn default() -> Self {
        Self::with_mode(Mode::Blocking)
    }
}

impl ImageCache {
    fn with_mode(mode: Mode) -> Self {
        Self {
            entries: HashMap::new(),
            mode,
            blocking: false,
            frame: 0,
            budget: BUDGET,
            keep_side: KEEP_SIDE,
        }
    }

    /// A cache that decodes on worker threads and calls `wake` whenever a picture is
    /// ready (from a worker thread), so the app draws again.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn background(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_mode(Mode::Background(pool::Pool::new(Box::new(wake))))
    }

    /// A cache that decodes one picture a frame; while [`ImageCache::busy`], the app
    /// should draw another frame.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn progressive() -> Self {
        Self::with_mode(Mode::Progressive {
            left: 1,
            deferred: false,
        })
    }

    /// Decodes right away while on (files and printing need every picture at once);
    /// returns what it was.
    pub fn set_blocking(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.blocking, on)
    }

    /// Whether pictures wanted lately are still being decoded.
    pub fn busy(&self) -> bool {
        match &self.mode {
            Mode::Blocking => false,
            Mode::Progressive { deferred, .. } => *deferred,
            #[cfg(not(target_arch = "wasm32"))]
            Mode::Background(pool) => pool.busy(),
        }
    }

    /// Starts a frame: takes in finished decodes, stops decoding pictures the last frame
    /// did not draw, and keeps memory within the budget.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        match &mut self.mode {
            Mode::Blocking => {}
            Mode::Progressive { left, deferred } => {
                *left = 1;
                *deferred = false;
            }
            #[cfg(not(target_arch = "wasm32"))]
            Mode::Background(pool) => {
                let (done, dropped) = pool.collect(self.frame - 1);
                for (id, ticket, result) in done {
                    if let Some(entry) = self.entries.get_mut(&id)
                        && entry.job == Some(ticket)
                    {
                        entry.job = None;
                        entry.set(result, id);
                    }
                }
                for (id, ticket) in dropped {
                    if let Some(entry) = self.entries.get_mut(&id)
                        && entry.job == Some(ticket)
                    {
                        entry.job = None;
                    }
                }
            }
        }
        // Pictures waiting to be decoded that are not wanted any more; asked for again
        // when drawn.
        self.entries
            .retain(|_, e| !e.levels.is_empty() || e.failed || e.job.is_some());
        self.trim();
    }

    /// The image for `id` with a longer side of at least `side` pixels where it can.
    /// While a bigger copy is decoded, a smaller one is given.
    pub fn get(&mut self, assets: &Assets, id: AssetId, side: f64) -> Picture<'_> {
        // A missing file may arrive later (pasted again, synced), so it is not cached.
        let Some(bytes) = assets.get(id) else {
            return Picture::Broken;
        };
        let frame = self.frame;
        let entry = self.entries.entry(id).or_default();
        if entry.failed {
            return Picture::Broken;
        }
        if !entry.covers(side) {
            let now = self.blocking
                || match &mut self.mode {
                    Mode::Blocking => true,
                    Mode::Progressive { left, deferred } => {
                        if *left > 0 {
                            *left -= 1;
                            true
                        } else {
                            *deferred = true;
                            false
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    Mode::Background(pool) => {
                        match entry.job {
                            Some(ticket) => pool.touch(ticket, frame),
                            None => entry.job = Some(pool.push(id, bytes.clone(), frame)),
                        }
                        false
                    }
                };
            if now {
                #[cfg(not(target_arch = "wasm32"))]
                if let (Some(ticket), Mode::Background(pool)) = (entry.job.take(), &self.mode) {
                    pool.cancel(ticket);
                }
                entry.set(decode(bytes), id);
            }
        }
        if entry.levels.is_empty() {
            entry.used = frame;
            return if entry.failed {
                Picture::Broken
            } else {
                Picture::Loading
            };
        }
        let i = entry.pick(side);
        let drawn = longer_side(&entry.levels[i].image);
        entry.used_side = if entry.used == frame {
            entry.used_side.max(drawn)
        } else {
            drawn
        };
        entry.used = frame;
        Picture::Ready(&entry.levels[i])
    }

    /// Drops the decoded image of `id`, e.g. a book's page turned away from: decoded pages
    /// are large, and a book has many.
    pub fn forget(&mut self, id: AssetId) {
        if let Some(_entry) = self.entries.remove(&id) {
            #[cfg(not(target_arch = "wasm32"))]
            if let (Some(ticket), Mode::Background(pool)) = (_entry.job, &self.mode) {
                pool.cancel(ticket);
            }
        }
    }

    /// Drops all decoded images (another board was opened).
    pub fn clear(&mut self) {
        self.entries.clear();
        #[cfg(not(target_arch = "wasm32"))]
        if let Mode::Background(pool) = &self.mode {
            pool.cancel_all();
        }
    }

    /// Bytes of decoded pixels kept.
    fn size(&self) -> usize {
        self.entries.values().map(Entry::size).sum()
    }

    /// Drops copies until within the budget: first those bigger than drawn (down to
    /// [`KEEP_SIDE`]), then the big copies of pictures not drawn in the last frame, then
    /// those pictures, then copies bigger than drawn at all; least lately drawn first.
    fn trim(&mut self) {
        let mut total = self.size();
        if total <= self.budget {
            return;
        }
        let (recent, keep_side) = (self.frame.saturating_sub(1), self.keep_side);
        let mut order: Vec<_> = self.entries.iter().map(|(id, e)| (e.used, *id)).collect();
        order.sort_unstable();
        for pass in 0..4 {
            for (_, id) in &order {
                if total <= self.budget {
                    return;
                }
                let Some(entry) = self.entries.get_mut(id) else {
                    continue;
                };
                let stale = entry.used < recent;
                let keep = match pass {
                    0 => entry.used_side.max(keep_side),
                    1 if stale => keep_side,
                    2 if stale => 0,
                    3 => entry.used_side,
                    _ => continue,
                };
                total -= entry.shrink(keep);
            }
        }
    }
}

impl Entry {
    /// Whether a copy at least `side` long is here, or the biggest there is.
    fn covers(&self, side: f64) -> bool {
        self.levels.first().is_some_and(|top| {
            let top = longer_side(&top.image);
            top == self.full || f64::from(top) >= side
        })
    }

    /// The smallest copy at least `side` long, or else the biggest one.
    fn pick(&self, side: f64) -> usize {
        self.levels
            .iter()
            .rposition(|l| f64::from(longer_side(&l.image)) >= side)
            .unwrap_or(0)
    }

    fn set(&mut self, result: Result<Vec<ImageBrush>, String>, id: AssetId) {
        match result {
            Ok(levels) => {
                self.full = levels.first().map_or(0, |l| longer_side(&l.image));
                self.levels = levels;
            }
            Err(err) => {
                log::warn!("cannot show image {id}: {err}");
                // A smaller copy is better than nothing; not retried either way.
                self.full = self.levels.first().map_or(0, |l| longer_side(&l.image));
                self.failed = self.levels.is_empty();
            }
        }
    }

    fn size(&self) -> usize {
        self.levels.iter().map(|l| l.image.data.data().len()).sum()
    }

    /// Drops copies longer than `keep` (all of them for 0, else at least one stays);
    /// returns the bytes freed.
    fn shrink(&mut self, keep: u32) -> usize {
        let before = self.size();
        if keep == 0 {
            self.levels.clear();
        } else {
            let over = self
                .levels
                .iter()
                .take_while(|l| longer_side(&l.image) > keep)
                .count()
                .min(self.levels.len().saturating_sub(1));
            self.levels.drain(..over);
        }
        before - self.size()
    }
}

fn longer_side(image: &ImageData) -> u32 {
    image.width.max(image.height)
}

/// The image in `bytes` and its smaller copies, biggest first.
fn decode(bytes: &[u8]) -> Result<Vec<ImageBrush>, String> {
    let pixels = pb_io::image::decode(bytes, MAX_SIDE).map_err(|e| e.to_string())?;
    Ok(levels(image_data(pixels.rgba, pixels.width, pixels.height)))
}

/// `image`, then each copy half as large as the one before, down to [`MIN_SIDE`].
fn levels(image: ImageData) -> Vec<ImageBrush> {
    let mut levels = vec![ImageBrush::new(image)];
    loop {
        let last = &levels[levels.len() - 1].image;
        if longer_side(last) / 2 < MIN_SIDE {
            return levels;
        }
        let half = halve(last);
        levels.push(ImageBrush::new(half));
    }
}

fn image_data(rgba: Vec<u8>, width: u32, height: u32) -> ImageData {
    ImageData {
        data: Blob::from(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    }
}

/// The image at half its size: each pixel averages a 2×2 block, colors weighted by alpha
/// so transparent pixels do not darken edges.
fn halve(image: &ImageData) -> ImageData {
    let (w, h) = (image.width as usize, image.height as usize);
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let src = image.data.data();
    let mut out = vec![0u8; hw * hh * 4];
    for y in 0..hh {
        for x in 0..hw {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                let p = &src[(sy * w + sx) * 4..][..4];
                let a = u32::from(p[3]);
                for c in 0..3 {
                    sum[c] += u32::from(p[c]) * a;
                }
                sum[3] += a;
            }
            let o = &mut out[(y * hw + x) * 4..][..4];
            for c in 0..3 {
                o[c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            o[3] = (sum[3] / 4) as u8;
        }
    }
    image_data(out, hw as u32, hh as u32)
}

/// Worker threads that decode pictures, most lately wanted first.
#[cfg(not(target_arch = "wasm32"))]
mod pool {
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};

    use pb_core::AssetId;
    use vello::peniko::ImageBrush;

    pub type Done = (AssetId, u64, Result<Vec<ImageBrush>, String>);

    pub struct Pool {
        shared: Arc<Shared>,
        next: u64,
        /// Whether the threads were started (on the first job).
        pub(super) started: bool,
    }

    struct Shared {
        queue: Mutex<Queue>,
        more: Condvar,
        wake: Box<dyn Fn() + Send + Sync>,
    }

    #[derive(Default)]
    struct Queue {
        jobs: Vec<Job>,
        done: Vec<Done>,
        /// Decodes under way.
        running: usize,
        closed: bool,
    }

    struct Job {
        id: AssetId,
        ticket: u64,
        bytes: Arc<[u8]>,
        /// The frame that last wanted it.
        frame: u64,
    }

    impl Pool {
        pub fn new(wake: Box<dyn Fn() + Send + Sync>) -> Self {
            Self {
                shared: Arc::new(Shared {
                    queue: Mutex::default(),
                    more: Condvar::new(),
                    wake,
                }),
                next: 0,
                started: false,
            }
        }

        fn queue(&self) -> MutexGuard<'_, Queue> {
            self.shared
                .queue
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        }

        /// Queues a decode; returns its ticket.
        pub fn push(&mut self, id: AssetId, bytes: Arc<[u8]>, frame: u64) -> u64 {
            if !self.started {
                self.started = true;
                // Leaves a core for the app itself.
                let threads = std::thread::available_parallelism()
                    .map_or(2, |n| n.get())
                    .saturating_sub(1)
                    .clamp(1, 4);
                for _ in 0..threads {
                    let shared = self.shared.clone();
                    let spawned = std::thread::Builder::new()
                        .name("pb-images".into())
                        .spawn(move || work(&shared));
                    if let Err(err) = spawned {
                        log::warn!("cannot start an image decoding thread: {err}");
                    }
                }
            }
            self.next += 1;
            let ticket = self.next;
            self.queue().jobs.push(Job {
                id,
                ticket,
                bytes,
                frame,
            });
            self.shared.more.notify_one();
            ticket
        }

        /// The job of `ticket` is wanted in `frame`, if it is still waiting.
        pub fn touch(&self, ticket: u64, frame: u64) {
            if let Some(job) = self.queue().jobs.iter_mut().find(|j| j.ticket == ticket) {
                job.frame = frame;
            }
        }

        pub fn cancel(&self, ticket: u64) {
            self.queue().jobs.retain(|j| j.ticket != ticket);
        }

        pub fn cancel_all(&self) {
            let mut queue = self.queue();
            queue.jobs.clear();
            queue.done.clear();
        }

        /// Finished decodes, and the waiting jobs dropped for not being wanted since
        /// frame `since` (by asset and ticket).
        pub fn collect(&self, since: u64) -> (Vec<Done>, Vec<(AssetId, u64)>) {
            let mut queue = self.queue();
            let done = std::mem::take(&mut queue.done);
            let mut dropped = Vec::new();
            queue.jobs.retain(|j| {
                let keep = j.frame >= since;
                if !keep {
                    dropped.push((j.id, j.ticket));
                }
                keep
            });
            (done, dropped)
        }

        pub fn busy(&self) -> bool {
            let queue = self.queue();
            !queue.jobs.is_empty() || !queue.done.is_empty() || queue.running > 0
        }
    }

    impl Drop for Pool {
        fn drop(&mut self) {
            self.queue().closed = true;
            self.shared.more.notify_all();
        }
    }

    fn work(shared: &Shared) {
        let lock = || {
            shared
                .queue
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        };
        loop {
            let job = {
                let mut queue = lock();
                loop {
                    if queue.closed {
                        return;
                    }
                    // Wanted by the latest frame, and in the order it drew them.
                    let next = queue
                        .jobs
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, j)| (j.frame, std::cmp::Reverse(j.ticket)))
                        .map(|(i, _)| i);
                    if let Some(i) = next {
                        queue.running += 1;
                        break queue.jobs.swap_remove(i);
                    }
                    queue = shared
                        .more
                        .wait(queue)
                        .unwrap_or_else(|poison| poison.into_inner());
                }
            };
            let result = super::decode(&job.bytes);
            {
                let mut queue = lock();
                queue.running -= 1;
                if queue.closed {
                    return;
                }
                queue.done.push((job.id, job.ticket, result));
            }
            (shared.wake)();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
        let rgba = [shade, 0, 0, 255].repeat((width * height) as usize);
        let pixels = pb_io::image::Pixels {
            width,
            height,
            rgba,
        };
        pb_io::image::encode_png(&pixels).unwrap()
    }

    fn side(picture: Picture<'_>) -> Option<u32> {
        match picture {
            Picture::Ready(brush) => Some(longer_side(&brush.image)),
            Picture::Loading => None,
            Picture::Broken => Some(0),
        }
    }

    fn top(cache: &ImageCache, id: AssetId) -> u32 {
        longer_side(&cache.entries[&id].levels[0].image)
    }

    /// Draws frames until `id` is no longer loading.
    #[cfg(not(target_arch = "wasm32"))]
    fn wait(cache: &mut ImageCache, assets: &Assets, id: AssetId) -> u32 {
        let start = Instant::now();
        loop {
            cache.begin_frame();
            if let Some(side) = side(cache.get(assets, id, 100.0)) {
                return side;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "never decoded");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn small_on_screen_images_use_small_copies() {
        let rgba = vec![255; 2000 * 1000 * 4];
        let mut entry = Entry::default();
        entry.set(Ok(levels(image_data(rgba, 2000, 1000))), AssetId::of(b""));
        let side = |e: &Entry, s: f64| longer_side(&e.levels[e.pick(s)].image);
        assert_eq!(side(&entry, 5000.0), 2000);
        assert_eq!(side(&entry, 1500.0), 2000);
        assert_eq!(side(&entry, 1000.0), 1000);
        assert_eq!(side(&entry, 300.0), 500);
        // Never smaller than MIN_SIDE.
        assert_eq!(side(&entry, 1.0), 125);
        assert_eq!(entry.levels[entry.pick(1.0)].image.height, 62);
        assert!(entry.covers(f64::INFINITY));
    }

    #[test]
    fn halving_keeps_colors_under_transparency() {
        // Red opaque pixel next to transparent black ones.
        let mut rgba = vec![0u8; 4 * 4];
        rgba[..4].copy_from_slice(&[255, 0, 0, 255]);
        let half = halve(&image_data(rgba, 2, 2));
        assert_eq!((half.width, half.height), (1, 1));
        assert_eq!(half.data.data(), &[255, 0, 0, 63]);
    }

    #[test]
    fn blocking_cache_decodes_right_away() {
        let mut assets = Assets::default();
        let id = assets.insert(png(300, 200, 10));
        let broken = assets.insert(b"not a picture".to_vec());
        let mut cache = ImageCache::default();
        cache.begin_frame();
        assert_eq!(side(cache.get(&assets, id, f64::INFINITY)), Some(300));
        assert_eq!(side(cache.get(&assets, id, 100.0)), Some(150));
        assert_eq!(side(cache.get(&assets, broken, 100.0)), Some(0));
        // A file not in the board yet.
        let missing = AssetId::of(b"elsewhere");
        assert_eq!(side(cache.get(&assets, missing, 100.0)), Some(0));
        assert!(!cache.busy());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_cache_loads_then_wakes() {
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        let mut cache = ImageCache::background(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let mut assets = Assets::default();
        let id = assets.insert(png(300, 200, 20));
        let broken = assets.insert(b"not a picture".to_vec());
        cache.begin_frame();
        assert!(matches!(cache.get(&assets, id, 100.0), Picture::Loading));
        assert!(matches!(
            cache.get(&assets, broken, 100.0),
            Picture::Loading
        ));
        assert_eq!(wait(&mut cache, &assets, id), 150);
        // A file that cannot be decoded is drawn as broken, not loading for ever.
        assert_eq!(wait(&mut cache, &assets, broken), 0);
        assert!(woken.load(Ordering::SeqCst) >= 2);
        assert!(!cache.busy());
        // Drawn from memory from now on.
        assert!(matches!(cache.get(&assets, id, 300.0), Picture::Ready(_)));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn exports_decode_right_away_in_the_background_cache() {
        let mut cache = ImageCache::background(|| {});
        let mut assets = Assets::default();
        let id = assets.insert(png(300, 200, 30));
        cache.begin_frame();
        let was = cache.set_blocking(true);
        assert_eq!(side(cache.get(&assets, id, f64::INFINITY)), Some(300));
        cache.set_blocking(was);
        assert!(!cache.busy());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pictures_scrolled_away_are_not_decoded() {
        let mut cache = ImageCache::background(|| {});
        // No workers: nothing gets decoded.
        if let Mode::Background(pool) = &mut cache.mode {
            pool.started = true;
        }
        let mut assets = Assets::default();
        let ids: Vec<_> = (0..40).map(|i| assets.insert(png(4, 4, i))).collect();
        cache.begin_frame();
        for &id in &ids {
            assert!(matches!(cache.get(&assets, id, 100.0), Picture::Loading));
        }
        assert!(cache.busy());
        // The next frame draws only the first; what else is waiting is dropped after it.
        cache.begin_frame();
        let _ = cache.get(&assets, ids[0], 100.0);
        cache.begin_frame();
        assert_eq!(cache.entries.len(), 1);
        cache.forget(ids[0]);
        assert!(!cache.busy());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn forgotten_pictures_stay_forgotten() {
        let mut cache = ImageCache::background(|| {});
        let mut assets = Assets::default();
        let id = assets.insert(png(300, 200, 40));
        cache.begin_frame();
        assert!(matches!(cache.get(&assets, id, 100.0), Picture::Loading));
        cache.forget(id);
        let start = Instant::now();
        while cache.busy() {
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
        cache.begin_frame();
        assert!(cache.entries.is_empty());
        // Decoded again when drawn again.
        assert_eq!(wait(&mut cache, &assets, id), 150);
        cache.forget(id);
        assert!(cache.entries.is_empty());
        cache.clear();
    }

    #[test]
    fn progressive_cache_decodes_one_a_frame() {
        let mut cache = ImageCache::progressive();
        let mut assets = Assets::default();
        let (a, b) = (
            assets.insert(png(100, 100, 1)),
            assets.insert(png(100, 100, 2)),
        );
        cache.begin_frame();
        assert!(matches!(cache.get(&assets, a, 100.0), Picture::Ready(_)));
        assert!(matches!(cache.get(&assets, b, 100.0), Picture::Loading));
        assert!(cache.busy());
        cache.begin_frame();
        assert!(matches!(cache.get(&assets, a, 100.0), Picture::Ready(_)));
        assert!(matches!(cache.get(&assets, b, 100.0), Picture::Ready(_)));
        assert!(!cache.busy());
    }

    #[test]
    fn over_the_budget_big_copies_go_first() {
        // Each picture: 512, 256, 128 and 64 pixels square, 1392640 bytes in all.
        let mut assets = Assets::default();
        let (a, b) = (
            assets.insert(png(512, 512, 1)),
            assets.insert(png(512, 512, 2)),
        );
        let mut cache = ImageCache {
            budget: 1_572_864,
            keep_side: 128,
            ..ImageCache::default()
        };
        cache.begin_frame();
        assert_eq!(side(cache.get(&assets, a, 512.0)), Some(512));
        cache.begin_frame();
        assert_eq!(side(cache.get(&assets, b, 100.0)), Some(128));
        // Copies of b bigger than drawn go; a, drawn bigger, keeps its.
        cache.begin_frame();
        assert_eq!((top(&cache, a), top(&cache, b)), (512, 128));
        assert!(cache.size() <= cache.budget);
        // Drawn bigger again: decoded again.
        assert_eq!(side(cache.get(&assets, b, 512.0)), Some(512));
        // Now a was not drawn lately: its big copies go.
        cache.begin_frame();
        assert_eq!((top(&cache, a), top(&cache, b)), (128, 512));
        // With no memory at all, pictures not drawn lately go, the ones drawn stay.
        let _ = cache.get(&assets, b, 300.0);
        cache.budget = 0;
        cache.begin_frame();
        assert!(!cache.entries.contains_key(&a) || cache.entries[&a].levels.is_empty());
        assert_eq!(top(&cache, b), 512);
        // Gone copies come back when drawn.
        assert_eq!(side(cache.get(&assets, a, 64.0)), Some(64));
    }
}
