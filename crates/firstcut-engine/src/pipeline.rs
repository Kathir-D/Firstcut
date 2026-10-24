//! The decode pipeline (docs/v2-plan.md §4.2): a priority window over the photos the user can
//! reach, cancellation when they move on, and a per-tier LRU cache under a memory budget.
//!
//! This is the policy half. It owns *what* should be decoded and *what* is worth keeping; the
//! app owns the threads and hands finished pixels back through [`Pipeline::complete`]. That
//! split is what makes the v1 audit defects testable without a window, a GPU or a real photo:
//!
//! - a cancelled decode must release its in-flight slot (`a_discarded_decode_releases_its_slot`),
//! - asking for pixels must never be recorded as having them
//!   (`a_request_is_not_a_cache_hit_until_pixels_land`),
//! - eviction is per tier and never touches the focus window
//!   (`eviction_never_crosses_tiers_or_the_focus_window`),
//! - the histogram is computed from the oriented display frame, not a second decode
//!   (`firstcut_decode::histogram`, tested there).
//!
//! Exact RAW is a tier of its own, never a size: a `Display` request can never be answered with
//! sensor pixels. It is held one photograph deep because a develop is ≈92 MB a frame, not because
//! it is slow.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use firstcut_decode::orientation_swaps_axes;
use firstcut_meta::meta::PhotoMeta;
use firstcut_types::PhotoId;

/// The decode tiers of §4.2. Cache keys are `(photo, tier)`, so tiers can never shadow each
/// other — the v1 defect where a display request could be answered with RAW pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Thumbnail,
    /// The camera's small `PRVW` JPEG at its native size. One decode is ~6–8 ms on the M1 Pro
    /// versus ~57–68 ms for the full embedded preview, which is what keeps a held arrow key from
    /// outrunning the loupe: every frame is painted with real pixels while the sharp display
    /// decode is still in flight.
    Preview,
    Display,
    ExactRaw,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Thumbnail => "thumbnail",
            Tier::Preview => "preview",
            Tier::Display => "display",
            Tier::ExactRaw => "exactRaw",
        }
    }
}

/// The filmstrip's reduction, v1's `prefetchPixels` default.
pub const THUMBNAIL_EDGE: u32 = 256;
/// What a display decode asks for before the window has reported a size (v1's
/// `fallbackDisplayEdge`).
pub const FALLBACK_DISPLAY_EDGE: u32 = 2048;
/// The navigation preview's target. The small `PRVW` JPEG is usually 1620×1080, so this returns
/// it untouched; a larger embedded preview is capped at 2048.
pub const PREVIEW_EDGE: u32 = 2048;

/// How far the sharp display tier is prefetched around the cursor. A display decode is ~57–68 ms
/// on an M1 Pro, so one worker cannot keep up with a 20 Hz held arrow (50 ms/frame); four workers
/// can (53–61 frames/s measured), but only if the work starts early enough. These are caps:
/// [`Pipeline::display_window`] shrinks them when the backing edge is large enough that the
/// frames would not fit the display budget.
const DISPLAY_PREFETCH_AHEAD: usize = 32;
const DISPLAY_PREFETCH_BEHIND: usize = 12;
/// The next batch's first frames are warmed before the cursor crosses the batch boundary. Without
/// this, every boundary is a cold start exactly when the user is moving fastest.
const DISPLAY_PREFETCH_NEXT_BATCH: usize = 8;

/// Bytes each tier may hold. Split from one total so Settings → Performance keeps a single
/// number, in sixteenths: thumbnails 1, navigation previews 1, displays 10 (five eighths), and
/// Exact RAW the last 4 (one 6000×4000 develop is ≈92 MB, so the quarter holds a handful and
/// the one-deep rule is what really bounds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierBudgets {
    pub thumbnail: usize,
    pub preview: usize,
    pub display: usize,
    pub exact_raw: usize,
}

impl TierBudgets {
    pub fn from_total(total: usize) -> Self {
        // Sixteenths: strip 1, navigation preview 1, display 10 (5/8), exact 4 (1/4). The
        // preview is only ~7 MB a frame, so this still holds dozens of them.
        Self { thumbnail: total / 16, preview: total / 16, display: total.saturating_mul(10) / 16, exact_raw: total / 4 }
    }

    pub fn of(self, tier: Tier) -> usize {
        match tier {
            Tier::Thumbnail => self.thumbnail,
            Tier::Preview => self.preview,
            Tier::Display => self.display,
            Tier::ExactRaw => self.exact_raw,
        }
    }
}

impl Default for TierBudgets {
    /// Four gigabytes until Settings → Performance hands over the real 40%-of-RAM number.
    fn default() -> Self {
        Self::from_total(4 * 1024 * 1024 * 1024)
    }
}

/// Where one photo's bytes live: the path the session opened it from and the metadata the scan
/// recorded (the embedded JPEG byte ranges, dimensions, orientation).
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub meta: PhotoMeta,
}

/// The loupe's physical (backing-pixel) size. Both axes are needed: a 3:2 photograph shown on a
/// 16:9 monitor does not fill the screen's long edge, so sizing from one "edge" number decodes
/// and uploads pixels nobody can see. `width`/`height` are zero before the first layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn long_edge(self) -> u32 {
        self.width.max(self.height)
    }

    fn area(self) -> u64 {
        u64::from(self.width).saturating_mul(u64::from(self.height))
    }
}

/// The longest edge a zoomed view actually displays: the fitted edge scaled by `zoom`, never
/// beyond the sensor's own long edge. This keeps a 2× laptop view from decoding all 24 MP.
pub fn zoomed_edge(fit: u32, full: u32, zoom: f32) -> u32 {
    let fit = fit.max(1);
    let full = full.max(fit);
    if !zoom.is_finite() || zoom <= 1.0 {
        return fit;
    }
    let scaled = f64::from(fit) * f64::from(zoom);
    if !scaled.is_finite() {
        return full;
    }
    (scaled.ceil() as u32).clamp(fit, full)
}

/// The longest edge, in backing pixels, that a photograph of `width`×`height` (sensor space,
/// with EXIF `orientation`) occupies when fitted inside `viewport`. This is the number the
/// display decoder should extract: a 3:2 photo on a 3840×2160 monitor is 3240 px wide, not
/// 3840, and a portrait photo on the same monitor is only 2160 px. The result never upscales a
/// photograph smaller than the viewer.
pub fn fit_edge(width: u32, height: u32, orientation: u8, viewport: Viewport) -> u32 {
    if viewport.is_empty() {
        return FALLBACK_DISPLAY_EDGE;
    }
    let (width, height) = if orientation_swaps_axes(orientation) { (height, width) } else { (width, height) };
    if width == 0 || height == 0 {
        return viewport.long_edge().max(1);
    }
    let scale = (f64::from(viewport.width) / f64::from(width)).min(f64::from(viewport.height) / f64::from(height)).min(1.0);
    let edge = (f64::from(width.max(height)) * scale).round();
    if !edge.is_finite() {
        return viewport.long_edge().max(1);
    }
    (edge as u32).max(1).min(viewport.long_edge().max(1))
}

/// What the user can reach next (v1's `FocusRequest`). `windows` is in priority order: the
/// current batch first, then the neighbouring batches nearest-first with next before previous.
/// Only the first three are guaranteed; the rest are optional prefetch.
#[derive(Clone, Debug, Default)]
pub struct Focus {
    pub windows: Vec<Vec<PhotoId>>,
    pub current: Option<PhotoId>,
    /// The loupe's backing pixels. The pipeline fits each photograph's own aspect into this
    /// rectangle; `0×0` before the window has a size falls back to [`FALLBACK_DISPLAY_EDGE`].
    pub viewport: Viewport,
    /// At 100% right now (actually `zoom > 1.01`): decode only the pixels the zoom displays.
    pub zoomed: bool,
    /// The loupe's zoom relative to fit (1.0 = the whole photo). A 2× view of a 1350 px fit
    /// needs 2700 px, not the 6000 px sensor; only the native zoom reaches the sensor edge.
    pub zoom: f32,
    /// Prefetch the frame the user is about to arrow to at the same zoom as well.
    pub zoom_lock: bool,
    pub exact_raw: bool,
}

/// One unit of work for the app's worker threads.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: PhotoId,
    pub tier: Tier,
    pub path: PathBuf,
    pub meta: PhotoMeta,
    /// The longest edge this job should produce (ignored for Exact RAW).
    pub target_edge: u32,
    /// Identifies this attempt. A result for a superseded generation is discarded, never stored.
    pub generation: u64,
}

/// Counters the debug HUD and `bench` read (the `PipelineStats` list in pipeline-api.md, minus
/// the fields that need the UI's zoom state).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub focus_misses: u64,
    pub thumbnail_cache_hits: u64,
    pub thumbnail_cache_misses: u64,
    pub thumbnail_decodes: u64,
    pub preview_cache_hits: u64,
    pub preview_cache_misses: u64,
    pub preview_decodes: u64,
    pub display_cache_hits: u64,
    pub display_cache_misses: u64,
    pub display_decodes: u64,
    pub exact_raw_decodes: u64,
    pub decode_failures: u64,
    /// Results that arrived for a request the pipeline had already cancelled.
    pub discarded_decodes: u64,
    pub cancellations: u64,
    pub pressure_sheds: u64,
}

struct Entry<V> {
    value: V,
    bytes: usize,
    /// The produced image's longest edge, so a zoomed request can ask for more later.
    edge: u32,
}

struct Pending {
    generation: u64,
}

struct Demand {
    key: Key,
    target_edge: u32,
    rank: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    id: PhotoId,
    tier: Tier,
}

/// The policy half of the decode engine. Generic over the cached value so the app can cache
/// `egui::ColorImage`s while the tests cache plain bytes, with the same scheduling, cancellation
/// and eviction rules on both.
pub struct Pipeline<V> {
    budgets: TierBudgets,
    bytes_of: fn(&V) -> usize,
    sources: HashMap<PhotoId, Source>,
    cache: HashMap<Key, Entry<V>>,
    /// One recency list per tier. A single list was the v1 defect: evicting a thumbnail could
    /// take a display bitmap with it.
    lru: HashMap<Tier, VecDeque<Key>>,
    pending: HashMap<Key, Pending>,
    /// The largest edge this key has ever produced. Once a decode returns the same (source is
    /// smaller than the request), asking again cannot help; this is how the pipeline settles.
    best_edge: HashMap<Key, u32>,
    failed: HashSet<Key>,
    bytes: HashMap<Tier, usize>,
    pinned: HashSet<Key>,
    /// The keys the current window wants. Eviction only ever removes keys outside this set, so
    /// a completed decode can never be evicted and immediately requested again (a busy loop).
    demanded: HashSet<Key>,
    demand: Vec<Demand>,
    focus: Focus,
    stats: Stats,
    next_generation: u64,
}

impl<V> Pipeline<V> {
    pub fn new(budgets: TierBudgets, bytes_of: fn(&V) -> usize) -> Self {
        Self {
            budgets,
            bytes_of,
            sources: HashMap::new(),
            cache: HashMap::new(),
            lru: HashMap::new(),
            pending: HashMap::new(),
            best_edge: HashMap::new(),
            failed: HashSet::new(),
            bytes: HashMap::new(),
            pinned: HashSet::new(),
            demanded: HashSet::new(),
            demand: Vec::new(),
            focus: Focus::default(),
            stats: Stats::default(),
            next_generation: 0,
        }
    }

    pub fn budgets(&self) -> TierBudgets {
        self.budgets
    }

    pub fn set_budgets(&mut self, budgets: TierBudgets) {
        self.budgets = budgets;
        for tier in [Tier::Thumbnail, Tier::Preview, Tier::Display, Tier::ExactRaw] {
            self.evict(tier);
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The app's catalog changed (a folder opened, a rescan reconciled). Entries whose photo is
    /// still present survive — a file landing in the folder must not blank the strip the user is
    /// in the middle of rating (v1's `open` fix).
    pub fn set_catalog(&mut self, sources: Vec<Source>) {
        self.update_catalog(sources);
    }

    pub fn update_catalog(&mut self, sources: Vec<Source>) {
        let next: HashMap<PhotoId, Source> = sources.into_iter().map(|source| (source.meta.id, source)).collect();
        self.cache.retain(|key, _| next.contains_key(&key.id));
        self.best_edge.retain(|key, _| next.contains_key(&key.id));
        self.failed.retain(|key| next.contains_key(&key.id));
        self.pending.retain(|key, _| next.contains_key(&key.id));
        for tier in [Tier::Thumbnail, Tier::Preview, Tier::Display, Tier::ExactRaw] {
            let mut kept = VecDeque::new();
            for key in self.lru.get(&tier).into_iter().flatten() {
                if next.contains_key(&key.id) {
                    kept.push_back(*key);
                }
            }
            self.lru.insert(tier, kept);
        }
        self.bytes.clear();
        for (key, entry) in &self.cache {
            *self.bytes.entry(key.tier).or_insert(0) += entry.bytes;
        }
        self.sources = next;
        self.rebuild_demand();
    }

    pub fn source(&self, id: PhotoId) -> Option<&Source> {
        self.sources.get(&id)
    }

    /// Reports where the user is. Recomputes the window, cancels work that fell out of it and
    /// releases those in-flight slots immediately; the late result is discarded on arrival.
    pub fn set_focus(&mut self, focus: Focus) {
        let previous_current = self.focus.current;
        self.focus = focus;
        if !self.focus.exact_raw {
            self.drop_tier(Tier::ExactRaw);
        }
        self.rebuild_demand();
        if self.focus.exact_raw {
            // The develop follows the cursor: leaving a photo drops its 92 MB at once.
            let keep = self.focus.current.map(|id| Key { id, tier: Tier::ExactRaw });
            self.retain_only(Tier::ExactRaw, keep);
        }
        self.pinned = self.pinned_keys();
        let stale: Vec<Key> = self.pending.keys().filter(|key| !self.demanded.contains(key)).copied().collect();
        for key in stale {
            self.pending.remove(&key);
            self.stats.cancellations = self.stats.cancellations.saturating_add(1);
        }
        if previous_current != self.focus.current
            && let Some(current) = self.focus.current
            && let Some(demand) = self.demand.iter().find(|demand| demand.key.id == current && demand.key.tier == Tier::Display)
            && !self.satisfies(demand)
        {
            self.stats.focus_misses = self.stats.focus_misses.saturating_add(1);
        }
        for tier in [Tier::Thumbnail, Tier::Preview, Tier::Display, Tier::ExactRaw] {
            self.evict(tier);
        }
    }

    pub fn focus(&self) -> &Focus {
        &self.focus
    }

    /// The next job to run, or `None` when the window is satisfied or already in flight. The
    /// visible frame is always first; the rest of the current batch follows, then the next
    /// batch, then the previous one.
    pub fn next_job(&mut self) -> Option<Job> {
        for demand in &self.demand {
            if self.pending.contains_key(&demand.key) || self.satisfies(demand) {
                continue;
            }
            let source = self.sources.get(&demand.key.id)?;
            self.next_generation = self.next_generation.saturating_add(1);
            let generation = self.next_generation;
            self.pending.insert(demand.key, Pending { generation });
            return Some(Job {
                id: demand.key.id,
                tier: demand.key.tier,
                path: source.path.clone(),
                meta: source.meta.clone(),
                target_edge: demand.target_edge,
                generation,
            });
        }
        None
    }

    /// Records a finished decode. `false` means the result was discarded (the focus moved on, or
    /// the catalog lost the photo) and the caller must not show it.
    pub fn complete(&mut self, job: &Job, value: V, edge: u32) -> bool {
        let key = Key { id: job.id, tier: job.tier };
        let Some(pending) = self.pending.get(&key) else {
            self.stats.discarded_decodes = self.stats.discarded_decodes.saturating_add(1);
            return false;
        };
        if pending.generation != job.generation {
            self.stats.discarded_decodes = self.stats.discarded_decodes.saturating_add(1);
            return false;
        }
        self.pending.remove(&key);
        self.best_edge.insert(key, self.best_edge.get(&key).copied().unwrap_or(0).max(edge));
        let bytes = (self.bytes_of)(&value);
        self.store(key, Entry { value, bytes, edge });
        match job.tier {
            Tier::Thumbnail => self.stats.thumbnail_decodes = self.stats.thumbnail_decodes.saturating_add(1),
            Tier::Preview => self.stats.preview_decodes = self.stats.preview_decodes.saturating_add(1),
            Tier::Display => self.stats.display_decodes = self.stats.display_decodes.saturating_add(1),
            Tier::ExactRaw => {
                self.stats.exact_raw_decodes = self.stats.exact_raw_decodes.saturating_add(1);
                // One photograph deep: a develop the user has left is 92 MB nobody can see.
                let keep = self.focus.current.map(|id| Key { id, tier: Tier::ExactRaw });
                self.retain_only(Tier::ExactRaw, keep);
            }
        }
        self.evict(job.tier);
        true
    }

    /// Records a failed decode against a job. The slot is released and the key is not retried
    /// until the catalog changes; a corrupt file costs one attempt, not one per frame.
    pub fn fail(&mut self, job: &Job) -> bool {
        let key = Key { id: job.id, tier: job.tier };
        let matches = self.pending.get(&key).is_some_and(|pending| pending.generation == job.generation);
        if !matches {
            self.stats.discarded_decodes = self.stats.discarded_decodes.saturating_add(1);
            return false;
        }
        self.pending.remove(&key);
        self.failed.insert(key);
        self.stats.decode_failures = self.stats.decode_failures.saturating_add(1);
        true
    }

    /// A cached value, touching its recency. `None` means "not decoded yet" — never a request
    /// recorded as a result.
    pub fn get(&mut self, id: PhotoId, tier: Tier) -> Option<&V> {
        let key = Key { id, tier };
        if self.cache.contains_key(&key) {
            self.touch(key);
            self.record_hit(tier);
            return self.cache.get(&key).map(|entry| &entry.value);
        }
        self.record_miss(tier);
        None
    }

    /// A cached value without touching recency or the stats. UI paint passes use this so
    /// reading what is on screen does not distort the LRU.
    pub fn peek(&self, id: PhotoId, tier: Tier) -> Option<&V> {
        self.cache.get(&Key { id, tier }).map(|entry| &entry.value)
    }

    pub fn contains(&self, id: PhotoId, tier: Tier) -> bool {
        self.cache.contains_key(&Key { id, tier })
    }

    pub fn cached_bytes(&self, tier: Tier) -> usize {
        self.bytes.get(&tier).copied().unwrap_or(0)
    }

    pub fn cached_entries(&self, tier: Tier) -> usize {
        self.cache.keys().filter(|key| key.tier == tier).count()
    }

    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    pub fn is_in_flight(&self, id: PhotoId, tier: Tier) -> bool {
        self.pending.contains_key(&Key { id, tier })
    }

    /// The v1 pressure handler: drop everything outside the focus window at once.
    pub fn shed_to_focus(&mut self) {
        let pinned = self.pinned.clone();
        let doomed: Vec<Key> = self.cache.keys().filter(|key| !pinned.contains(key)).copied().collect();
        for key in doomed {
            self.remove(key);
        }
        self.stats.pressure_sheds = self.stats.pressure_sheds.saturating_add(1);
    }

    // ---- internals -------------------------------------------------------------------------

    fn record_hit(&mut self, tier: Tier) {
        match tier {
            Tier::Thumbnail => self.stats.thumbnail_cache_hits = self.stats.thumbnail_cache_hits.saturating_add(1),
            Tier::Preview => self.stats.preview_cache_hits = self.stats.preview_cache_hits.saturating_add(1),
            Tier::Display => self.stats.display_cache_hits = self.stats.display_cache_hits.saturating_add(1),
            Tier::ExactRaw => {}
        }
    }

    fn record_miss(&mut self, tier: Tier) {
        match tier {
            Tier::Thumbnail => self.stats.thumbnail_cache_misses = self.stats.thumbnail_cache_misses.saturating_add(1),
            Tier::Preview => self.stats.preview_cache_misses = self.stats.preview_cache_misses.saturating_add(1),
            Tier::Display => self.stats.display_cache_misses = self.stats.display_cache_misses.saturating_add(1),
            Tier::ExactRaw => {}
        }
    }

    fn satisfies(&self, demand: &Demand) -> bool {
        if self.failed.contains(&demand.key) {
            return true;
        }
        let Some(entry) = self.cache.get(&demand.key) else { return false };
        if demand.key.tier == Tier::ExactRaw {
            return true;
        }
        let best = self.best_edge.get(&demand.key).copied().unwrap_or(entry.edge);
        entry.edge >= demand.target_edge || entry.edge >= best
    }

    fn touch(&mut self, key: Key) {
        let order = self.lru.entry(key.tier).or_default();
        if let Some(position) = order.iter().position(|candidate| *candidate == key) {
            order.remove(position);
        }
        order.push_back(key);
    }

    fn store(&mut self, key: Key, entry: Entry<V>) {
        if let Some(old) = self.cache.remove(&key) {
            if let Some(bytes) = self.bytes.get_mut(&key.tier) {
                *bytes = bytes.saturating_sub(old.bytes);
            }
            if let Some(order) = self.lru.get_mut(&key.tier)
                && let Some(position) = order.iter().position(|candidate| *candidate == key)
            {
                order.remove(position);
            }
        }
        *self.bytes.entry(key.tier).or_insert(0) += entry.bytes;
        self.cache.insert(key, entry);
        self.lru.entry(key.tier).or_default().push_back(key);
    }

    fn remove(&mut self, key: Key) {
        if let Some(entry) = self.cache.remove(&key)
            && let Some(bytes) = self.bytes.get_mut(&key.tier)
        {
            *bytes = bytes.saturating_sub(entry.bytes);
        }
        if let Some(order) = self.lru.get_mut(&key.tier)
            && let Some(position) = order.iter().position(|candidate| *candidate == key)
        {
            order.remove(position);
        }
    }

    fn drop_tier(&mut self, tier: Tier) {
        let keys: Vec<Key> = self.cache.keys().filter(|key| key.tier == tier).copied().collect();
        for key in keys {
            self.remove(key);
        }
        let pending: Vec<Key> = self.pending.keys().filter(|key| key.tier == tier).copied().collect();
        for key in pending {
            self.pending.remove(&key);
            self.stats.cancellations = self.stats.cancellations.saturating_add(1);
        }
    }

    fn retain_only(&mut self, tier: Tier, keep: Option<Key>) {
        let doomed: Vec<Key> = self.cache.keys().filter(|key| key.tier == tier && Some(**key) != keep).copied().collect();
        for key in doomed {
            self.remove(key);
        }
        let pending: Vec<Key> = self.pending.keys().filter(|key| key.tier == tier && Some(**key) != keep).copied().collect();
        for key in pending {
            self.pending.remove(&key);
            self.stats.cancellations = self.stats.cancellations.saturating_add(1);
        }
    }

    fn evict(&mut self, tier: Tier) {
        let budget = self.budgets.of(tier);
        loop {
            let used = self.bytes.get(&tier).copied().unwrap_or(0);
            if used <= budget {
                return;
            }
            let victim = {
                let Some(order) = self.lru.get(&tier) else { return };
                order.iter().copied().find(|key| !self.demanded.contains(key))
            };
            match victim {
                Some(key) => self.remove(key),
                // Everything left is wanted: over budget beats a blank viewer, and evicting a
                // demanded key would only schedule it again next frame.
                None => return,
            }
        }
    }

    fn pinned_keys(&self) -> HashSet<Key> {
        let mut pinned = HashSet::new();
        for ids in self.focus.windows.iter().take(3) {
            for id in ids {
                pinned.insert(Key { id: *id, tier: Tier::Thumbnail });
            }
        }
        if let Some(current_batch) = self.focus.windows.first() {
            for id in current_batch {
                pinned.insert(Key { id: *id, tier: Tier::Preview });
            }
        }
        if let Some(current_batch) = self.focus.windows.first() {
            for id in current_batch {
                pinned.insert(Key { id: *id, tier: Tier::Display });
            }
        }
        if let Some(current) = self.focus.current {
            pinned.insert(Key { id: current, tier: Tier::ExactRaw });
        }
        pinned
    }

    /// How many neighbours the display tier can prefetch in each direction without letting the
    /// demand set itself exceed the display budget. A fitted frame can never occupy more than
    /// the viewport rectangle, so the rectangle's area is the exact upper bound; using a square
    /// of the long edge would shrink a 4K/5K window by the monitor's unused aspect margin.
    fn display_window(&self, viewport: Viewport) -> (usize, usize, usize) {
        if self.focus.zoomed {
            return (3, 2, 0);
        }
        let frame_bytes = if viewport.is_empty() {
            u64::from(FALLBACK_DISPLAY_EDGE).saturating_mul(u64::from(FALLBACK_DISPLAY_EDGE)).saturating_mul(4)
        } else {
            viewport.area().saturating_mul(4)
        };
        let slots = usize::try_from((self.budgets.display as u64) / frame_bytes.max(1)).unwrap_or(usize::MAX);
        // At a 2048 px edge there is room for the full head; at 5–6K the same eight-frame
        // heads would crowd out the current batch, so the head shrinks with the budget.
        let head = (slots / 8).clamp(2, DISPLAY_PREFETCH_NEXT_BATCH);
        // Leave room for the visible frame, the strip and both neighbouring batch heads.
        let wanted = slots.saturating_sub(4 + 2 * head).clamp(8, DISPLAY_PREFETCH_AHEAD + DISPLAY_PREFETCH_BEHIND);
        let ahead = (wanted.saturating_mul(3) / 4).clamp(4, DISPLAY_PREFETCH_AHEAD);
        let behind = wanted.saturating_sub(ahead).clamp(2, DISPLAY_PREFETCH_BEHIND);
        (ahead, behind, head)
    }

    fn rebuild_demand(&mut self) {
        let mut demand: Vec<Demand> = Vec::new();
        let current = self.focus.current;
        let viewport = self.focus.viewport;
        let current_batch = self.focus.windows.first().cloned().unwrap_or_default();
        let current_position = current.and_then(|id| current_batch.iter().position(|candidate| *candidate == id));

        if let Some(current) = current {
            let source = self.sources.get(&current);
            if let Some(source) = source {
                let full = source.meta.width.max(source.meta.height);
                // The navigation preview is first: it is ~6–8 ms, so a held arrow key always has
                // a real frame to paint while the ~57–68 ms display decode is still in flight.
                demand.push(Demand { key: Key { id: current, tier: Tier::Preview }, target_edge: PREVIEW_EDGE, rank: 0 });
                let fitted = fit_edge(source.meta.width, source.meta.height, source.meta.orientation, viewport);
                demand.push(Demand {
                    key: Key { id: current, tier: Tier::Display },
                    target_edge: if self.focus.zoomed { zoomed_edge(fitted, full, self.focus.zoom) } else { fitted },
                    rank: 1,
                });
                demand.push(Demand { key: Key { id: current, tier: Tier::Thumbnail }, target_edge: THUMBNAIL_EDGE, rank: 10 });
                if self.focus.exact_raw {
                    // Last, after the visible frame, its preview and the strip: an exact develop
                    // takes every core for ~0.4 s, so it must never preempt navigation.
                    demand.push(Demand { key: Key { id: current, tier: Tier::ExactRaw }, target_edge: 0, rank: 50 });
                }
            }
        }

        // Preview prefetch: the current batch only, four behind and eight ahead. One decode is
        // ~9 ms, so a burst of arrows has warm pixels for the whole next screenful.
        if let Some(position) = current_position {
            for offset in -4i64..=8 {
                if offset == 0 {
                    continue;
                }
                let Some(index) = position.checked_add_signed(offset as isize) else { continue };
                let Some(id) = current_batch.get(index).copied() else { continue };
                if !self.sources.contains_key(&id) {
                    continue;
                }
                demand.push(Demand {
                    key: Key { id, tier: Tier::Preview },
                    target_edge: PREVIEW_EDGE,
                    rank: 100 + (offset.unsigned_abs() as u32) * 2 + u32::from(offset < 0),
                });
            }
        }

        // Display prefetch: a long forward-biased window, bounded by the display budget. At
        // 20 Hz the cursor needs a frame every 50 ms; a ~57–68 ms decode cannot answer that on
        // demand, but four workers can stay ahead if they are given the next second of frames
        // while the current one is painted. The window is deliberately after the current
        // batch's strip thumbnails (rank 1_000) and before the next batch's strip (rank 2_000).
        let (ahead, behind, head) = self.display_window(viewport);
        if let Some(position) = current_position {
            for offset in -(behind as i64)..=(ahead as i64) {
                if offset == 0 {
                    continue;
                }
                let Some(index) = position.checked_add_signed(offset as isize) else { continue };
                let Some(id) = current_batch.get(index).copied() else { continue };
                let Some(source) = self.sources.get(&id) else { continue };
                let full = source.meta.width.max(source.meta.height);
                let zoom_prefetch = self.focus.zoom_lock && offset == 1;
                let fitted = fit_edge(source.meta.width, source.meta.height, source.meta.orientation, viewport);
                let target_edge = if self.focus.zoomed || zoom_prefetch { zoomed_edge(fitted, full, self.focus.zoom) } else { fitted };
                demand.push(Demand {
                    key: Key { id, tier: Tier::Display },
                    target_edge,
                    rank: 1_500 + (offset.unsigned_abs() as u32) * 2 + u32::from(offset < 0),
                });
            }
        }
        // Warm the heads of both neighbouring batches too: a batch boundary must not turn a
        // held arrow into a cold start in either direction. These sit just before the
        // neighbouring batches' strip thumbnails.
        if !self.focus.zoomed {
            for (window, base_rank) in [(1usize, 1_800u32), (2usize, 1_900u32)] {
                let Some(batch) = self.focus.windows.get(window) else { continue };
                for (index, id) in batch.iter().take(head).enumerate() {
                    let Some(source) = self.sources.get(id) else { continue };
                    if self.focus.current == Some(*id) {
                        continue;
                    }
                    let fitted = fit_edge(source.meta.width, source.meta.height, source.meta.orientation, viewport);
                    demand.push(Demand { key: Key { id: *id, tier: Tier::Display }, target_edge: fitted, rank: base_rank + index as u32 });
                }
            }
        }

        for (window_index, ids) in self.focus.windows.iter().enumerate() {
            for (index, id) in ids.iter().enumerate() {
                let Some(_source) = self.sources.get(id) else { continue };
                let rank = if window_index == 0 {
                    match current_position {
                        Some(position) => 1_000 + (index.abs_diff(position) as u32) * 2 + u32::from(index < position),
                        None => 1_000 + index as u32,
                    }
                } else if window_index == 1 {
                    2_000 + index as u32
                } else {
                    3_000 + index as u32
                };
                demand.push(Demand { key: Key { id: *id, tier: Tier::Thumbnail }, target_edge: THUMBNAIL_EDGE, rank });
            }
        }

        demand.sort_by_key(|demand| demand.rank);
        self.demanded = demand.iter().map(|demand| demand.key).collect();
        self.demand = demand;
    }
}

#[cfg(test)]
mod tests {
    //! The four v1 `ImageProvider` audit defects, as named tests, plus the window rules.
    use super::*;

    fn photo(id: u64, width: u32, height: u32) -> Source {
        Source {
            path: PathBuf::from(format!("/photos/IMG_{id:04}.CR3")),
            meta: PhotoMeta {
                id: PhotoId(id),
                rel_path: format!("IMG_{id:04}.CR3"),
                companions: Vec::new(),
                kind: firstcut_meta::meta::FileKind::Raw(firstcut_meta::meta::RawFormat::Cr3),
                file_size: 1,
                capture_time: None,
                shutter_count: None,
                file_number: None,
                camera_make: None,
                camera_model: None,
                camera_serial: None,
                lens_model: None,
                focal_length_mm: None,
                exposure_time_s: None,
                f_number: None,
                iso: None,
                exposure_comp_ev: None,
                metering_mode: None,
                drive_mode: None,
                shutter_mode: None,
                orientation: 1,
                width,
                height,
                af: None,
                preview: None,
                full_preview: None,
                device: None,
                ino: None,
                warnings: Vec::new(),
            },
        }
    }

    // The signature has to match `bytes_of: fn(&V) -> usize` with `V = Vec<u8>`.
    #[allow(clippy::ptr_arg)]
    fn bytes(value: &Vec<u8>) -> usize {
        value.len()
    }

    fn catalog() -> Vec<Source> {
        (1..=8).map(|id| photo(id, 6000, 4000)).collect()
    }

    fn wide_catalog() -> Vec<Source> {
        (1..=80).map(|id| photo(id, 6000, 4000)).collect()
    }

    fn focus_on(current: u64) -> Focus {
        Focus {
            windows: vec![vec![PhotoId(1), PhotoId(2), PhotoId(3), PhotoId(4)], vec![PhotoId(5), PhotoId(6)], vec![PhotoId(7), PhotoId(8)]],
            current: Some(PhotoId(current)),
            viewport: Viewport::new(2048, 1365),
            ..Focus::default()
        }
    }

    fn pipeline() -> Pipeline<Vec<u8>> {
        let mut pipeline = Pipeline::new(TierBudgets::default(), bytes);
        pipeline.set_catalog(catalog());
        pipeline
    }

    /// Runs the current window to completion with payload `value`.
    fn settle(pipeline: &mut Pipeline<Vec<u8>>, payload: &[u8]) {
        while let Some(job) = pipeline.next_job() {
            assert!(pipeline.complete(&job, payload.to_vec(), if job.tier == Tier::ExactRaw { 6000 } else { job.target_edge }));
        }
    }

    #[test]
    fn the_visible_frame_is_scheduled_first_then_neighbours_then_the_other_batches() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(2));
        let first = pipeline.next_job().expect("a fresh window has work");
        assert_eq!((first.id, first.tier), (PhotoId(2), Tier::Preview), "the fast navigation preview must land first");
        assert!(pipeline.complete(&first, vec![0; 4], first.target_edge));
        let second = pipeline.next_job().expect("the sharp display follows");
        assert_eq!((second.id, second.tier), (PhotoId(2), Tier::Display), "the loupe's sharp frame follows the preview");
        assert!(pipeline.complete(&second, vec![0; 4], second.target_edge));
        let mut order = Vec::new();
        while let Some(job) = pipeline.next_job() {
            order.push((job.id.0, job.tier));
            pipeline.complete(&job, vec![0; 4], job.target_edge);
            if order.len() > 30 {
                break;
            }
        }
        assert!(order.iter().any(|(id, tier)| *id == 2 && *tier == Tier::Thumbnail), "the current photo's strip frame follows");
        let first_next = order.iter().position(|(id, tier)| *id == 5 && *tier == Tier::Thumbnail).expect("next batch is fetched");
        let first_previous = order.iter().position(|(id, tier)| *id == 7 && *tier == Tier::Thumbnail).expect("previous batch is fetched");
        assert!(first_next < first_previous, "next is warmed before previous at equal distance");
    }

    /// A held arrow key paints the fast `PRVW` preview and defers the exact develop behind the
    /// visible frame and the strip. This is the scheduling half of that promise; the app adds the
    /// settle timer, and the benchmark row measures the ~7 ms preview against a 33 ms key repeat.
    #[test]
    fn the_navigation_preview_lands_before_the_display_and_the_exact_develop() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(2).with_exact_raw());

        let preview = pipeline.next_job().expect("the current preview is first");
        assert_eq!((preview.id, preview.tier), (PhotoId(2), Tier::Preview));
        assert!(pipeline.complete(&preview, vec![0; 8], preview.target_edge));

        let display = pipeline.next_job().expect("the sharp display follows");
        assert_eq!((display.id, display.tier), (PhotoId(2), Tier::Display));
        assert!(pipeline.complete(&display, vec![0; 8], display.target_edge));

        let strip = pipeline.next_job().expect("the strip frame follows the visible frame");
        assert_eq!((strip.id, strip.tier), (PhotoId(2), Tier::Thumbnail));
        assert!(pipeline.complete(&strip, vec![0; 8], strip.target_edge));

        let exact = pipeline.next_job().expect("the exact develop is queued after the visible work");
        assert_eq!((exact.id, exact.tier), (PhotoId(2), Tier::ExactRaw), "the 0.34 s develop must never preempt navigation");
    }

    #[test]
    fn the_navigation_preview_is_prefetched_ahead_but_never_answers_a_display_request() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(2));
        settle(&mut pipeline, &[0; 8]);
        assert!(pipeline.contains(PhotoId(1), Tier::Preview), "one behind is warm");
        for id in 3..=4 {
            assert!(pipeline.contains(PhotoId(id), Tier::Preview), "the rest of the current batch is warm");
        }
        assert!(!pipeline.contains(PhotoId(5), Tier::Preview), "preview prefetch stays inside the current batch");
        assert!(pipeline.contains(PhotoId(3), Tier::Display), "the sharp prefetch is separate");
        assert_eq!(pipeline.get(PhotoId(3), Tier::Preview).map(Vec::len), Some(8));
        assert_eq!(pipeline.get(PhotoId(3), Tier::Display).map(Vec::len), Some(8));
    }

    /// A held arrow key at 20 Hz gets a ~1.6 s sharp-frame lead, not just three neighbours. The
    /// four decode workers need that lead because one display decode (~57–68 ms) is slower than the
    /// 50 ms key interval; the preview tier still paints immediately while the lead is built.
    #[test]
    fn the_display_window_prefetches_the_next_second_of_a_held_arrow() {
        let mut pipeline = Pipeline::new(TierBudgets::default(), bytes);
        pipeline.set_catalog(wide_catalog());
        let first: Vec<PhotoId> = (1..=40).map(PhotoId).collect();
        let second: Vec<PhotoId> = (41..=80).map(PhotoId).collect();
        pipeline.set_focus(Focus {
            windows: vec![first, second],
            current: Some(PhotoId(20)),
            viewport: Viewport::new(2048, 1365),
            ..Focus::default()
        });
        settle(&mut pipeline, &[0; 8]);

        for id in 8..=40 {
            assert!(pipeline.contains(PhotoId(id), Tier::Display), "current batch neighbour {id} is warm");
        }
        assert!(!pipeline.contains(PhotoId(7), Tier::Display), "the window is bounded, not the whole catalog");
        for id in 41..=48 {
            assert!(pipeline.contains(PhotoId(id), Tier::Display), "the next batch's head is warm too");
        }
        assert!(!pipeline.contains(PhotoId(49), Tier::Display), "only the next batch's head is prefetched");
    }

    #[test]
    fn the_display_target_fits_the_photo_aspect_into_the_physical_viewport() {
        // 3:2 on a 16:9 4K panel: height is the constraint, so the extracted long edge is
        // 3240 px — the screen's 3840 would decode and upload 18% extra width nobody sees.
        assert_eq!(fit_edge(6000, 4000, 1, Viewport::new(3840, 2160)), 3240);
        // A 1x laptop pane: 900 px tall, so 1350 px wide.
        assert_eq!(fit_edge(6000, 4000, 1, Viewport::new(1440, 900)), 1350);
        // A 14" Retina pane at 2x: 1964 px tall, 2946 px wide.
        assert_eq!(fit_edge(6000, 4000, 1, Viewport::new(3024, 1964)), 2946);
        // Portrait photo on the same 4K panel: only the height fits.
        assert_eq!(fit_edge(4000, 6000, 1, Viewport::new(3840, 2160)), 2160);
        // Orientation 6 swaps the axes before fitting.
        assert_eq!(fit_edge(6000, 4000, 6, Viewport::new(3840, 2160)), 2160);
        // A small frame is never upscaled.
        assert_eq!(fit_edge(1600, 1000, 1, Viewport::new(3840, 2160)), 1600);
        assert_eq!(fit_edge(6000, 4000, 1, Viewport::default()), FALLBACK_DISPLAY_EDGE);
    }

    #[test]
    fn a_zoom_decodes_the_pixels_the_zoom_shows_not_the_whole_sensor() {
        assert_eq!(zoomed_edge(1350, 6000, 1.0), 1350);
        assert_eq!(zoomed_edge(1350, 6000, 2.0), 2700);
        assert_eq!(zoomed_edge(1350, 6000, 12.0), 6000);
        assert_eq!(zoomed_edge(3240, 6000, 1.5), 4860);
        assert_eq!(zoomed_edge(3240, 6000, 2.0), 6000, "never beyond the sensor's long edge");
    }

    #[test]
    fn the_current_display_job_carries_the_fitted_target() {
        let mut pipeline = pipeline();
        pipeline.set_focus(Focus {
            windows: vec![vec![PhotoId(1)]],
            current: Some(PhotoId(1)),
            viewport: Viewport::new(3840, 2160),
            ..Focus::default()
        });
        // The preview is rank 0; the display is rank 1 and must ask for 3240, not 3840.
        let preview = pipeline.next_job().expect("preview first");
        assert_eq!(preview.tier, Tier::Preview);
        assert!(pipeline.complete(&preview, vec![0; 4], preview.target_edge));
        let display = pipeline.next_job().expect("display second");
        assert_eq!(display.tier, Tier::Display);
        assert_eq!(display.target_edge, 3240, "a 3:2 photo on a 3840×2160 pane is 3240 px wide");
    }

    #[test]
    fn a_large_retina_edge_shrinks_the_display_window_to_the_budget() {
        let pipeline = pipeline();
        assert_eq!(pipeline.display_window(Viewport::new(2048, 1365)), (32, 12, 8));
        let (ahead, behind, head) = pipeline.display_window(Viewport::new(6000, 4000));
        assert!(ahead < 32 && behind < 12 && head < 8, "a 6000 px frame is ~144 MB; the window must shrink: {ahead}/{behind}/{head}");
    }

    /// Defect 1: `store(_:_:)`'s discard branch dropped the cancel without releasing the in-flight
    /// slot, so the decoded photos stopped arriving. A late result for a cancelled request must
    /// free the booking and never enter the cache.
    #[test]
    fn a_discarded_decode_releases_its_slot() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(1));
        let job = pipeline.next_job().expect("the visible frame is queued");
        assert_eq!(pipeline.in_flight(), 1);

        // The user arrows away before the worker answers; the request is now out of the window.
        pipeline.set_focus(focus_on(8));
        assert_eq!(pipeline.in_flight(), 0, "cancelling frees the slot immediately");
        assert!(pipeline.stats().cancellations >= 1);

        pipeline.set_focus(focus_on(1));
        assert!(!pipeline.complete(&job, vec![7; 16], 8), "a superseded generation is discarded");
        assert_eq!(pipeline.stats().discarded_decodes, 1);
        assert_eq!(pipeline.in_flight(), 0, "the discard must not leak a booking");
        assert!(!pipeline.contains(PhotoId(1), Tier::Display), "a cancelled result is never shown");
        assert!(pipeline.next_job().is_some(), "asking again after the discard schedules a fresh decode");
    }

    /// Defect 2: the request used to be recorded as a result, so the first ask claimed a cache
    /// hit and the caller painted a frame that did not exist.
    #[test]
    fn a_request_is_not_a_cache_hit_until_pixels_land() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(1));
        assert!(pipeline.get(PhotoId(1), Tier::Display).is_none(), "nothing has decoded yet");
        assert_eq!(pipeline.stats().display_cache_hits, 0);
        assert_eq!(pipeline.stats().display_cache_misses, 1);

        let preview = pipeline.next_job().expect("the preview is queued first");
        assert_eq!(preview.tier, Tier::Preview);
        assert!(pipeline.get(PhotoId(1), Tier::Preview).is_none(), "queued is not decoded");
        assert!(pipeline.complete(&preview, vec![1; 16], preview.target_edge));
        assert_eq!(pipeline.get(PhotoId(1), Tier::Preview).map(Vec::len), Some(16));

        let job = pipeline.next_job().expect("the display request is queued");
        assert_eq!(job.tier, Tier::Display);
        assert!(pipeline.get(PhotoId(1), Tier::Display).is_none(), "queued is not decoded");
        assert_eq!(pipeline.stats().display_cache_hits, 0);

        assert!(pipeline.complete(&job, vec![3; 32], 2));
        assert_eq!(pipeline.get(PhotoId(1), Tier::Display).map(Vec::len), Some(32));
        assert_eq!(pipeline.stats().display_cache_hits, 1);
    }

    /// Defect 3: one shared LRU let a thumbnail pass evict display bitmaps (and vice versa). The
    /// tiers now have their own budgets and recency lists; a focus-window entry is never a victim.
    #[test]
    fn eviction_never_crosses_tiers_or_the_focus_window() {
        let mut pipeline = Pipeline::new(TierBudgets { thumbnail: 64, preview: 1024, display: 1024, exact_raw: 1024 }, bytes);
        pipeline.set_catalog(catalog());
        pipeline.set_focus(focus_on(1));
        settle(&mut pipeline, &[0; 16]);
        assert!(pipeline.contains(PhotoId(1), Tier::Display), "the loupe is decoded");

        // Churn thumbnails outside whatever window is current until the thumbnail budget spills.
        for id in 3..=8 {
            pipeline.set_focus(Focus {
                windows: vec![vec![PhotoId(id)]],
                current: Some(PhotoId(id)),
                viewport: Viewport::new(2048, 1365),
                ..Focus::default()
            });
            settle(&mut pipeline, &[0; 40]);
        }
        assert!(pipeline.cached_bytes(Tier::Thumbnail) <= 64, "the thumbnail tier stays inside its own budget");
        assert!(pipeline.contains(PhotoId(1), Tier::Display), "thumbnail eviction must not take a display bitmap with it");
        assert!(pipeline.contains(PhotoId(8), Tier::Thumbnail), "the current window's thumbnail survives its own budget");
        pipeline.set_focus(focus_on(1));
        assert!(pipeline.contains(PhotoId(1), Tier::Display));
    }

    #[test]
    fn exact_raw_is_one_photograph_deep_and_never_answers_a_display_request() {
        let mut pipeline = pipeline();
        let mut focus = focus_on(1);
        focus.exact_raw = true;
        pipeline.set_focus(focus.clone());
        // Drain until the develop is the only pending thing left, then run it.
        let mut develop = None;
        while let Some(job) = pipeline.next_job() {
            if job.tier == Tier::ExactRaw {
                develop = Some(job);
                break;
            }
            pipeline.complete(&job, vec![0; 8], job.target_edge);
        }
        let develop = develop.expect("Exact RAW is queued while the setting is on");
        assert!(pipeline.complete(&develop, vec![9; 64], 6000));
        assert!(pipeline.contains(PhotoId(1), Tier::ExactRaw));
        assert_eq!(pipeline.cached_entries(Tier::ExactRaw), 1);
        assert_eq!(pipeline.get(PhotoId(1), Tier::Display).map(|value| value.len()), Some(8), "display never returns the develop");

        pipeline.set_focus(focus_on(2).with_exact_raw());
        assert!(!pipeline.contains(PhotoId(1), Tier::ExactRaw), "leaving the photo drops the 92 MB develop");
        assert_eq!(pipeline.cached_entries(Tier::ExactRaw), 0);

        pipeline.set_focus(focus_on(2)); // exact RAW off
        assert_eq!(pipeline.cached_entries(Tier::ExactRaw), 0);
    }

    #[test]
    fn memory_pressure_sheds_everything_outside_the_focus_window() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(1));
        settle(&mut pipeline, &[0; 8]);
        assert!(pipeline.contains(PhotoId(1), Tier::Display));
        // Make photos 1 and 2 the whole focus, then shed.
        pipeline.set_focus(Focus {
            windows: vec![vec![PhotoId(1), PhotoId(2)]],
            current: Some(PhotoId(2)),
            viewport: Viewport::new(2048, 1365),
            ..Focus::default()
        });
        pipeline.shed_to_focus();
        assert!(pipeline.contains(PhotoId(1), Tier::Display), "the focus window survives a pressure shed");
        assert!(!pipeline.contains(PhotoId(3), Tier::Thumbnail), "everything outside it does not");
        assert_eq!(pipeline.stats().pressure_sheds, 1);
    }

    #[test]
    fn a_failed_decode_is_not_requested_again() {
        let mut pipeline = pipeline();
        pipeline.set_focus(focus_on(1));
        let job = pipeline.next_job().expect("queued");
        assert_eq!(job.tier, Tier::Preview);
        assert!(pipeline.fail(&job));
        assert_eq!(pipeline.in_flight(), 0);
        pipeline.set_focus(focus_on(1));
        let display = pipeline.next_job().expect("a failed preview does not block the sharp frame");
        assert_eq!(display.tier, Tier::Display);
        assert!(pipeline.complete(&display, vec![0; 4], display.target_edge));
        while let Some(job) = pipeline.next_job() {
            assert!(!(job.id == PhotoId(1) && job.tier == Tier::Preview), "a failed decode must not loop");
            pipeline.complete(&job, vec![0; 4], job.target_edge);
        }
    }

    #[test]
    fn a_rescan_keeps_the_entries_whose_photos_stayed() {
        let mut pipeline = pipeline();
        let mut catalog_with_extra = catalog();
        catalog_with_extra.push(photo(9, 6000, 4000));
        pipeline.set_catalog(catalog_with_extra);
        pipeline.set_focus(focus_on(1));
        settle(&mut pipeline, &[0; 8]);
        let display_bytes = pipeline.cached_bytes(Tier::Display);
        // Photo 9 is outside the focus window and was never demanded, so its departure is
        // genuinely unrelated to the loupe's cached display frames.
        let mut next = catalog();
        next.push(photo(9, 6000, 4000));
        next.retain(|source| source.meta.id != PhotoId(9));
        pipeline.update_catalog(next);
        assert!(pipeline.contains(PhotoId(1), Tier::Display), "an unrelated file leaving must not blank the loupe");
        assert!(!pipeline.contains(PhotoId(9), Tier::Thumbnail));
        assert_eq!(pipeline.cached_bytes(Tier::Display), display_bytes, "the bytes still present are not recounted");
    }

    impl Focus {
        fn with_exact_raw(mut self) -> Self {
            self.exact_raw = true;
            self
        }
    }
}
