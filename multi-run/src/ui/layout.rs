//! Grid dimension math, tile rects, pagination, and focus movement.
//! DESIGN.md section 6 ("Tile geometry") and section 7 (navigation
//! keybindings) are the source of truth. Every function here is pure: no
//! ratatui `Frame`, no terminal IO, only `ratatui::layout::Rect` as a value
//! type. Callers (`ui/mod.rs`, and later `app.rs`) own all state; this module
//! only computes.
//!
//! `ViewMode`, `Pane`, and `Focus` are defined here rather than in `model.rs`
//! or `app.rs` because `compute()` and the focus-movement functions below are
//! their first and, for now, only consumers. `app.rs` (a later milestone)
//! should import them as `crate::ui::layout::{ViewMode, Pane, Focus}` rather
//! than redefine them.

use ratatui::layout::Rect;

/// Minimum inner content width of a tile: 22 columns of content plus a 2-cell
/// border.
pub const MIN_TILE_W: u16 = 24;
/// Minimum inner content height of a tile: 5 rows of content plus a 2-cell
/// border.
pub const MIN_TILE_H: u16 = 7;

/// Which of the two top-level layouts the app is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Grid,
    ListDetail,
}

/// Which region has keyboard focus. Meaningful only in `ViewMode::ListDetail`;
/// Grid view has a single focused tile and no separate panes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    List,
    Detail,
}

/// The focused job, plus (in list/detail view) which pane owns the keyboard.
/// Grid pagination is tracked separately, on `App::page`, because it applies
/// across the whole group rather than to one focus value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Focus {
    pub job: usize,
    pub pane: Pane,
}

/// Choose a column count that keeps tiles close to a 2:1 cell aspect, which
/// reads as roughly square once a terminal cell's own aspect (taller than
/// wide) is accounted for. `n == 0` returns `(1, 1)`; there is nothing to lay
/// out, and the caller should not draw any tiles, but a sane pair avoids a
/// divide-by-zero in `clamp`.
pub fn grid_dims(n: usize, area: Rect) -> (u16, u16) {
    if n == 0 {
        return (1, 1);
    }
    let ideal = ideal_cols(n, area);

    // The aspect-ideal column count can make tiles narrower or shorter than
    // the minimum even when some other column count for the same `n` fits.
    // Prefer the fitting count nearest the ideal, so a group only ever
    // paginates when no arrangement at all fits.
    let mut best: Option<u16> = None;
    for cols in 1..=n as u16 {
        let rows = (n as u16).div_ceil(cols);
        if meets_minimum(cols, rows, area)
            && best.is_none_or(|b| ideal.abs_diff(cols) < ideal.abs_diff(b))
        {
            best = Some(cols);
        }
    }

    let cols = best.unwrap_or(ideal);
    (cols, (n as u16).div_ceil(cols))
}

/// Column count that puts tiles closest to a 2:1 cell aspect, ignoring the
/// minimum tile size. Split out so `grid_dims` can measure how far a fitting
/// arrangement sits from the shape we actually wanted.
fn ideal_cols(n: usize, area: Rect) -> u16 {
    let w = area.width.max(1) as f32;
    let h = area.height.max(1) as f32;
    let cols = ((n as f32 * w) / (2.0 * h)).sqrt().ceil();
    (cols as u16).clamp(1, n as u16)
}

/// Split `total` into `n` near-equal parts summing exactly to `total`, with
/// any remainder distributed one unit at a time to the first slices. Mirrors
/// how ratatui's own `Layout` divides space, so tiles tile the area exactly
/// with no gaps and no overlap.
fn split_extent(total: u16, n: u16) -> Vec<u16> {
    if n == 0 {
        return Vec::new();
    }
    let base = total / n;
    let rem = total % n;
    (0..n)
        .map(|i| if i < rem { base + 1 } else { base })
        .collect()
}

/// Lay out exactly `count` tiles in row-major order across a `cols` x `rows`
/// grid inside `area`. `count` may be less than `cols * rows` (an incomplete
/// last row on the final page); the missing cells are simply not produced.
fn tile_rects(cols: u16, rows: u16, count: usize, area: Rect) -> Vec<Rect> {
    let col_widths = split_extent(area.width, cols);
    let row_heights = split_extent(area.height, rows);

    let mut x_offsets = Vec::with_capacity(cols as usize);
    let mut x = area.x;
    for w in &col_widths {
        x_offsets.push(x);
        x += w;
    }
    let mut y_offsets = Vec::with_capacity(rows as usize);
    let mut y = area.y;
    for h in &row_heights {
        y_offsets.push(y);
        y += h;
    }

    (0..count)
        .map(|i| {
            let row = i / cols as usize;
            let col = i % cols as usize;
            Rect {
                x: x_offsets[col],
                y: y_offsets[row],
                width: col_widths[col],
                height: row_heights[row],
            }
        })
        .collect()
}

/// Does a `cols` x `rows` grid over `area` give every tile at least the
/// minimum size? Uses floor division, which is exactly the smallest tile
/// `split_extent` can produce (the remainder only ever adds a unit, never
/// subtracts one).
fn meets_minimum(cols: u16, rows: u16, area: Rect) -> bool {
    cols > 0 && rows > 0 && area.width / cols >= MIN_TILE_W && area.height / rows >= MIN_TILE_H
}

/// Resolved grid layout for one page of a group's jobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub view: ViewMode,
    pub cols: u16,
    pub rows: u16,
    /// How many jobs fit on one grid page. Constant across pages for a given
    /// `(n, area, view)`; only `tiles` (and its length) vary with `page`.
    pub per_page: usize,
    pub pages: usize,
    /// Inner rect per visible tile, in the same order as the page's job
    /// slice (`jobs[page * per_page .. page * per_page + tiles.len()]`).
    pub tiles: Vec<Rect>,
}

/// Largest job count per page, at most `n`, whose grid still meets the
/// minimum tile size in `area`. Searches downward from `n` because fewer
/// tiles on a page only ever grows tile size, so the first count that fits is
/// the best (largest) one. Falls back to 1 if even a single tile does not
/// meet the minimum, per DESIGN.md section 6: "a hopelessly small terminal
/// still shows one tile rather than dividing by zero."
fn choose_per_page(n: usize, area: Rect) -> usize {
    if n == 0 {
        return 0;
    }
    for candidate in (1..=n).rev() {
        let (cols, rows) = grid_dims(candidate, area);
        if meets_minimum(cols, rows, area) {
            return candidate;
        }
    }
    1
}

/// Compute the grid geometry for one page of `n` jobs inside `area`.
///
/// In `ViewMode::ListDetail` there is no grid and no pagination: the list
/// itself scrolls, so this returns a single page holding all `n` jobs with
/// `cols = 1, rows = n.max(1)` (the list pane draws its own rows; these
/// values exist so callers do not need to special-case list view when asking
/// "how many pages"). `tiles` is left empty in this mode; `ui/mod.rs` and
/// `ui/tile.rs` own list/detail drawing, not this module.
///
/// `page` is clamped into range; an out-of-range page (e.g. after jobs were
/// dismissed and the page count shrank) is treated as the last valid page.
pub fn compute(n: usize, area: Rect, view: ViewMode, page: usize) -> Geometry {
    if view == ViewMode::ListDetail {
        return Geometry {
            view,
            cols: 1,
            rows: n.max(1) as u16,
            per_page: n,
            pages: if n == 0 { 0 } else { 1 },
            tiles: Vec::new(),
        };
    }

    if n == 0 {
        return Geometry {
            view,
            cols: 0,
            rows: 0,
            per_page: 0,
            pages: 0,
            tiles: Vec::new(),
        };
    }

    let per_page = choose_per_page(n, area);
    let pages = n.div_ceil(per_page);
    let page = page.min(pages.saturating_sub(1));

    let start = page * per_page;
    let end = (start + per_page).min(n);
    let count = end - start;

    let (cols, rows) = grid_dims(count, area);
    let tiles = tile_rects(cols, rows, count, area);

    Geometry {
        view,
        cols,
        rows,
        per_page,
        pages,
        tiles,
    }
}

/// Index of the first job on `page`, given the geometry's `per_page`.
pub fn page_start(page: usize, per_page: usize) -> usize {
    page * per_page
}

/// Which page a job index falls on, given `per_page`. `per_page == 0` (an
/// empty group) trivially returns page 0.
fn page_of(job: usize, per_page: usize) -> usize {
    if per_page == 0 { 0 } else { job / per_page }
}

/// Focus the next job in row-major order, wrapping from the last job back to
/// the first. Stepping past the last tile on a grid page lands on the first
/// tile of the next page (and wraps from the last page to the first), so `j`
/// alone walks the whole group without the user tracking pages. Returns
/// `(job, page)`; `n == 0` returns `(0, 0)`.
pub fn focus_next(job: usize, per_page: usize, n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let next = if job + 1 >= n { 0 } else { job + 1 };
    (next, page_of(next, per_page))
}

/// Focus the previous job in row-major order, wrapping from the first job to
/// the last. See `focus_next` for the pagination behaviour this mirrors.
pub fn focus_prev(job: usize, per_page: usize, n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let prev = if job == 0 { n - 1 } else { job - 1 };
    (prev, page_of(prev, per_page))
}

/// Focus the tile to the left of `job` on its current grid page. A no-op at
/// the left edge of a row: left/right movement stays within one row rather
/// than wrapping to the previous row's rightmost tile, which would jump
/// across the screen unpredictably.
pub fn focus_left(job: usize, page: usize, geo: &Geometry) -> usize {
    let local = job - page_start(page, geo.per_page);
    if geo.cols == 0 || local % geo.cols as usize == 0 {
        job
    } else {
        job - 1
    }
}

/// Focus the tile to the right of `job` on its current grid page. A no-op at
/// the right edge of a row, and also when the next slot in the row has no
/// tile at all (an incomplete last row on the final page).
pub fn focus_right(job: usize, page: usize, geo: &Geometry) -> usize {
    let local = job - page_start(page, geo.per_page);
    let at_row_edge = geo.cols == 0 || (local + 1) % geo.cols as usize == 0;
    if at_row_edge || local + 1 >= geo.tiles.len() {
        job
    } else {
        job + 1
    }
}

/// Focus the first job overall: index 0, page 0.
pub fn focus_first() -> (usize, usize) {
    (0, 0)
}

/// Focus the last job overall.
pub fn focus_last(n: usize, per_page: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let job = n - 1;
    (job, page_of(job, per_page))
}

/// Focus a specific job index directly (`Alt-1`..`Alt-9`), clamped into
/// range. `n == 0` returns `(0, 0)`.
pub fn focus_index(idx: usize, n: usize, per_page: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let job = idx.min(n - 1);
    (job, page_of(job, per_page))
}

/// Advance to the next grid page, wrapping from the last page to the first.
/// `pages == 0` returns 0.
pub fn next_page(page: usize, pages: usize) -> usize {
    if pages == 0 { 0 } else { (page + 1) % pages }
}

/// Move to the previous grid page, wrapping from the first page to the last.
/// `pages == 0` returns 0.
pub fn prev_page(page: usize, pages: usize) -> usize {
    if pages == 0 {
        0
    } else {
        (page + pages - 1) % pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREAS: [Rect; 6] = [
        Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        },
        Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 60,
        },
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 12,
        },
        Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 3,
        },
        Rect {
            x: 2,
            y: 1,
            width: 90,
            height: 30,
        },
    ];

    // --- grid_dims ---------------------------------------------------

    #[test]
    fn grid_dims_n_zero_does_not_panic() {
        let (cols, rows) = grid_dims(0, AREAS[0]);
        assert_eq!((cols, rows), (1, 1));
    }

    #[test]
    fn grid_dims_n_one_is_one_by_one() {
        assert_eq!(grid_dims(1, AREAS[0]), (1, 1));
    }

    #[test]
    fn grid_dims_never_exceeds_n_columns() {
        for area in AREAS {
            for n in 1..=24usize {
                let (cols, rows) = grid_dims(n, area);
                assert!(cols as usize <= n, "cols {cols} > n {n} for area {area:?}");
                assert!(rows >= 1);
                assert!(
                    (rows as usize) * (cols as usize) >= n,
                    "grid too small for n={n}"
                );
            }
        }
    }

    #[test]
    fn grid_dims_three_jobs_is_not_forced_square() {
        // 3 jobs on a wide area should prefer 2 or 3 columns over a lone column.
        let (cols, rows) = grid_dims(3, AREAS[0]);
        assert!(
            cols >= 2,
            "expected >= 2 cols for 3 jobs on a wide area, got {cols}"
        );
        assert!(cols * rows >= 3);
    }

    #[test]
    fn grid_dims_does_not_panic_on_zero_sized_area() {
        let zero = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let (cols, rows) = grid_dims(5, zero);
        assert!(cols >= 1 && rows >= 1);
    }

    // --- compute: bounds and overlap, table-driven --------------------

    fn assert_no_overlap_and_in_bounds(area: Rect, tiles: &[Rect]) {
        for r in tiles {
            assert!(
                r.x >= area.x
                    && r.y >= area.y
                    && r.x + r.width <= area.x + area.width
                    && r.y + r.height <= area.y + area.height,
                "tile {r:?} escapes area {area:?}"
            );
        }
        for i in 0..tiles.len() {
            for j in (i + 1)..tiles.len() {
                let a = tiles[i];
                let b = tiles[j];
                let overlap_x = a.x < b.x + b.width && b.x < a.x + a.width;
                let overlap_y = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(!(overlap_x && overlap_y), "tiles {a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn compute_tiles_never_overlap_and_stay_in_bounds() {
        for area in AREAS {
            for n in 1..=24usize {
                let geo = compute(n, area, ViewMode::Grid, 0);
                assert_no_overlap_and_in_bounds(area, &geo.tiles);
                // Every job must land on some page.
                let total_on_pages: usize = (0..geo.pages)
                    .map(|p| compute(n, area, ViewMode::Grid, p).tiles.len())
                    .sum();
                assert_eq!(
                    total_on_pages, n,
                    "pages did not partition all {n} jobs, area {area:?}"
                );
            }
        }
    }

    #[test]
    fn compute_single_job() {
        let geo = compute(1, AREAS[0], ViewMode::Grid, 0);
        assert_eq!(geo.tiles.len(), 1);
        assert_eq!(geo.pages, 1);
        assert_eq!(
            geo.tiles[0],
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24
            }
        );
    }

    #[test]
    fn compute_two_jobs() {
        let geo = compute(2, AREAS[0], ViewMode::Grid, 0);
        assert_eq!(geo.tiles.len(), 2);
        assert_eq!(geo.pages, 1);
    }

    #[test]
    fn compute_three_jobs_non_square() {
        let geo = compute(3, AREAS[0], ViewMode::Grid, 0);
        assert_eq!(geo.tiles.len(), 3);
        assert_eq!(geo.pages, 1);
        // Not a single column: at least one row has more than one tile.
        assert!(geo.cols >= 2);
    }

    #[test]
    fn compute_degenerate_area_returns_exactly_one_tile_per_page() {
        // 10x3 is smaller than even one MIN_TILE_W x MIN_TILE_H tile.
        let area = AREAS[4];
        let geo = compute(5, area, ViewMode::Grid, 0);
        assert_eq!(geo.per_page, 1);
        assert_eq!(geo.tiles.len(), 1);
        assert_eq!(geo.pages, 5);
    }

    #[test]
    fn compute_many_jobs_paginates_when_minimum_size_forces_it() {
        // 40x12 area: MIN_TILE_W=24, MIN_TILE_H=7 means at most one column and
        // one row fit comfortably for a large n, forcing pagination.
        let geo = compute(20, AREAS[3], ViewMode::Grid, 0);
        assert!(geo.per_page < 20, "expected pagination on a small area");
        assert!(geo.pages > 1);
    }

    #[test]
    fn compute_out_of_range_page_clamps_to_last() {
        let geo_last = compute(10, AREAS[3], ViewMode::Grid, 999);
        let geo_valid = compute(10, AREAS[3], ViewMode::Grid, geo_last.pages - 1);
        assert_eq!(geo_last.tiles.len(), geo_valid.tiles.len());
    }

    #[test]
    fn compute_zero_jobs_is_empty_and_does_not_panic() {
        let geo = compute(0, AREAS[0], ViewMode::Grid, 0);
        assert_eq!(geo.tiles.len(), 0);
        assert_eq!(geo.pages, 0);
        assert_eq!(geo.per_page, 0);
    }

    #[test]
    fn compute_list_detail_has_one_page_and_no_tiles() {
        let geo = compute(7, AREAS[0], ViewMode::ListDetail, 0);
        assert_eq!(geo.pages, 1);
        assert_eq!(geo.per_page, 7);
        assert!(geo.tiles.is_empty());
    }

    // --- focus movement -----------------------------------------------

    #[test]
    fn focus_next_wraps_around_single_page() {
        let geo = compute(4, AREAS[0], ViewMode::Grid, 0);
        let (job, page) = focus_next(3, geo.per_page, 4);
        assert_eq!((job, page), (0, 0));
    }

    #[test]
    fn focus_prev_wraps_around_single_page() {
        let geo = compute(4, AREAS[0], ViewMode::Grid, 0);
        let (job, page) = focus_prev(0, geo.per_page, 4);
        assert_eq!((job, page), (3, 0));
    }

    #[test]
    fn focus_next_advances_page_at_boundary() {
        // Force pagination: small area, many jobs.
        let area = AREAS[3];
        let n = 20;
        let geo0 = compute(n, area, ViewMode::Grid, 0);
        assert!(geo0.pages > 1, "test assumes pagination");
        let last_on_page0 = geo0.per_page - 1;
        let (job, page) = focus_next(last_on_page0, geo0.per_page, n);
        assert_eq!(job, geo0.per_page);
        assert_eq!(page, 1);
    }

    #[test]
    fn focus_prev_retreats_page_at_boundary() {
        let area = AREAS[3];
        let n = 20;
        let geo0 = compute(n, area, ViewMode::Grid, 0);
        assert!(geo0.pages > 1, "test assumes pagination");
        let (job, page) = focus_prev(geo0.per_page, geo0.per_page, n);
        assert_eq!(job, geo0.per_page - 1);
        assert_eq!(page, 0);
    }

    #[test]
    fn focus_next_single_job_stays_put() {
        assert_eq!(focus_next(0, 1, 1), (0, 0));
    }

    #[test]
    fn focus_next_zero_jobs_does_not_panic() {
        assert_eq!(focus_next(0, 0, 0), (0, 0));
        assert_eq!(focus_prev(0, 0, 0), (0, 0));
    }

    // 60x30 is chosen because it makes grid_dims produce exactly 2 columns
    // for both n=3 and n=4, which the row/edge tests below depend on.
    const TWO_COL_AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 30,
    };

    #[test]
    fn focus_left_right_move_within_row() {
        let geo = compute(4, TWO_COL_AREA, ViewMode::Grid, 0);
        assert_eq!(geo.cols, 2, "test assumes a 2-column grid for 4 jobs");
        // Row-major: 0 1 / 2 3
        assert_eq!(focus_right(0, 0, &geo), 1);
        assert_eq!(focus_left(1, 0, &geo), 0);
        assert_eq!(focus_right(2, 0, &geo), 3);
        assert_eq!(focus_left(3, 0, &geo), 2);
    }

    #[test]
    fn focus_left_at_row_start_is_a_no_op() {
        let geo = compute(4, TWO_COL_AREA, ViewMode::Grid, 0);
        assert_eq!(focus_left(0, 0, &geo), 0);
        assert_eq!(focus_left(2, 0, &geo), 2);
    }

    #[test]
    fn focus_right_at_row_end_is_a_no_op() {
        let geo = compute(4, TWO_COL_AREA, ViewMode::Grid, 0);
        assert_eq!(focus_right(1, 0, &geo), 1);
        assert_eq!(focus_right(3, 0, &geo), 3);
    }

    #[test]
    fn focus_right_into_missing_last_row_tile_is_a_no_op() {
        // 3 jobs, 2 columns: row 1 has only column 0. Right from job 2 must
        // not step into a nonexistent job 3.
        let geo = compute(3, TWO_COL_AREA, ViewMode::Grid, 0);
        assert_eq!(geo.cols, 2, "test assumes a 2-column grid for 3 jobs");
        assert_eq!(focus_right(2, 0, &geo), 2);
    }

    #[test]
    fn focus_single_job_left_right_are_no_ops() {
        let geo = compute(1, AREAS[0], ViewMode::Grid, 0);
        assert_eq!(focus_left(0, 0, &geo), 0);
        assert_eq!(focus_right(0, 0, &geo), 0);
    }

    #[test]
    fn focus_first_and_last() {
        assert_eq!(focus_first(), (0, 0));
        let geo = compute(20, AREAS[3], ViewMode::Grid, 0);
        assert!(geo.pages > 1, "test assumes pagination");
        let (job, page) = focus_last(20, geo.per_page);
        assert_eq!(job, 19);
        assert_eq!(page, geo.pages - 1);
    }

    #[test]
    fn focus_last_zero_jobs() {
        assert_eq!(focus_last(0, 4), (0, 0));
    }

    #[test]
    fn focus_index_clamps_into_range() {
        let geo = compute(20, AREAS[3], ViewMode::Grid, 0);
        assert_eq!(focus_index(999, 20, geo.per_page), (19, geo.pages - 1));
        assert_eq!(focus_index(0, 20, geo.per_page), (0, 0));
        assert_eq!(focus_index(0, 0, 4), (0, 0));
    }

    // --- page navigation -----------------------------------------------

    #[test]
    fn next_prev_page_wrap() {
        assert_eq!(next_page(0, 3), 1);
        assert_eq!(next_page(2, 3), 0);
        assert_eq!(prev_page(0, 3), 2);
        assert_eq!(prev_page(1, 3), 0);
    }

    #[test]
    fn next_prev_page_zero_pages_does_not_panic() {
        assert_eq!(next_page(0, 0), 0);
        assert_eq!(prev_page(0, 0), 0);
    }

    #[test]
    fn page_start_matches_per_page_multiplication() {
        assert_eq!(page_start(0, 8), 0);
        assert_eq!(page_start(2, 8), 16);
    }
}
