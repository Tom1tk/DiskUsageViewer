//! Squarified treemap layout (Bruls, Huizing and van Wijk, 2000). Port of `DiskUsageViewer\treemap.py`.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

const ZERO: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 0.0,
    h: 0.0,
};

/// One rect per size, in the same order, tiling `bounds`. Sizes must be positive, largest first;
/// callers drop zero-size entries before calling.
pub fn layout(sizes: &[f64], bounds: Rect) -> Vec<Rect> {
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return vec![ZERO; sizes.len()];
    }
    let scale = bounds.w * bounds.h / total;
    let areas: Vec<f64> = sizes.iter().map(|size| size * scale).collect();
    let mut rects = Vec::with_capacity(sizes.len());
    let mut rest = bounds;
    let mut start = 0;
    while start < areas.len() {
        let end = row_end(&areas, start, rest.w.min(rest.h));
        let row = &areas[start..end];
        rest = if rest.w >= rest.h {
            place_column(row, rest, &mut rects)
        } else {
            place_strip(row, rest, &mut rects)
        };
        start = end;
    }
    rects
}

/// Index one past the last area that still improves the row's worst aspect ratio.
fn row_end(areas: &[f64], start: usize, short: f64) -> usize {
    let mut end = start + 1;
    while end < areas.len() && worst(&areas[start..=end], short) <= worst(&areas[start..end], short)
    {
        end += 1;
    }
    end
}

/// Lays the row as a column along the left edge and returns the remaining box.
fn place_column(row: &[f64], rest: Rect, out: &mut Vec<Rect>) -> Rect {
    let col_w = row.iter().sum::<f64>() / rest.h;
    let mut cy = rest.y;
    for area in row {
        let h = area / col_w;
        out.push(Rect {
            x: rest.x,
            y: cy,
            w: col_w,
            h,
        });
        cy += h;
    }
    Rect {
        x: rest.x + col_w,
        y: rest.y,
        w: rest.w - col_w,
        h: rest.h,
    }
}

/// Lays the row as a strip along the top edge and returns the remaining box.
fn place_strip(row: &[f64], rest: Rect, out: &mut Vec<Rect>) -> Rect {
    let row_h = row.iter().sum::<f64>() / rest.w;
    let mut cx = rest.x;
    for area in row {
        let w = area / row_h;
        out.push(Rect {
            x: cx,
            y: rest.y,
            w,
            h: row_h,
        });
        cx += w;
    }
    Rect {
        x: rest.x,
        y: rest.y + row_h,
        w: rest.w,
        h: rest.h - row_h,
    }
}

/// Worst aspect ratio of the row when it is laid along a side of length `short`.
fn worst(row: &[f64], short: f64) -> f64 {
    let total: f64 = row.iter().sum();
    let short_sq = short * short;
    row.iter()
        .map(|r| (short_sq * r / (total * total)).max(total * total / (short_sq * r)))
        .fold(f64::NEG_INFINITY, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect {
        x: 10.0,
        y: 20.0,
        w: 300.0,
        h: 200.0,
    };

    const SIZES: [f64; 20] = [
        100.0, 80.0, 60.0, 40.0, 30.0, 20.0, 15.0, 10.0, 8.0, 5.0, 4.0, 3.0, 2.0, 2.0, 1.0, 1.0,
        1.0, 1.0, 1.0, 1.0,
    ];

    fn area(r: &Rect) -> f64 {
        r.w * r.h
    }

    fn overlap(a: &Rect, b: &Rect) -> f64 {
        let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
        let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
        w.max(0.0) * h.max(0.0)
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(layout(&[], BOUNDS).is_empty());
    }

    #[test]
    fn zero_total_gives_zero_rects() {
        let rects = layout(&[0.0, 0.0, 0.0], BOUNDS);
        assert_eq!(rects, vec![ZERO; 3]);
    }

    #[test]
    fn zero_bounds_gives_zero_rects() {
        let flat = Rect { w: 0.0, ..BOUNDS };
        assert_eq!(layout(&[3.0, 1.0], flat), vec![ZERO; 2]);
    }

    #[test]
    fn rects_tile_the_bounds() {
        let rects = layout(&SIZES, BOUNDS);
        let sum: f64 = rects.iter().map(area).sum();
        let expected = area(&BOUNDS);
        assert!((sum - expected).abs() / expected < 1e-6);
        for r in &rects {
            assert!(r.x >= BOUNDS.x - 1e-9 && r.x + r.w <= BOUNDS.x + BOUNDS.w + 1e-9);
            assert!(r.y >= BOUNDS.y - 1e-9 && r.y + r.h <= BOUNDS.y + BOUNDS.h + 1e-9);
        }
    }

    #[test]
    fn rects_do_not_overlap() {
        let rects = layout(&SIZES, BOUNDS);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(overlap(a, b) < 1e-9);
            }
        }
    }

    #[test]
    fn areas_are_proportional_to_sizes() {
        let rects = layout(&SIZES, BOUNDS);
        let total: f64 = SIZES.iter().sum();
        for (r, size) in rects.iter().zip(SIZES) {
            let expected = size / total * area(&BOUNDS);
            assert!((area(r) - expected).abs() / expected < 1e-6);
        }
    }

    #[test]
    fn aspect_ratio_stays_below_four() {
        for r in layout(&SIZES, BOUNDS) {
            assert!((r.w / r.h).max(r.h / r.w) < 4.0, "{r:?}");
        }
    }

    #[test]
    fn matches_python_reference_layout() {
        let expected = [
            (0.0, 0.0, 3.0, 2.0),
            (0.0, 2.0, 3.0, 2.0),
            (3.0, 0.0, 1.7142857142857142, 2.3333333333333335),
            (
                4.714285714285714,
                0.0,
                1.2857142857142856,
                2.3333333333333335,
            ),
            (
                3.0,
                2.3333333333333335,
                1.2000000000000002,
                1.6666666666666665,
            ),
            (
                4.2,
                2.3333333333333335,
                1.2000000000000002,
                1.6666666666666665,
            ),
            (
                5.4,
                2.3333333333333335,
                0.5999999999999996,
                1.6666666666666676,
            ),
        ];
        let rects = layout(
            &[6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0],
            Rect {
                x: 0.0,
                y: 0.0,
                w: 6.0,
                h: 4.0,
            },
        );
        for (r, (x, y, w, h)) in rects.iter().zip(expected) {
            assert!((r.x - x).abs() < 1e-9, "{r:?}");
            assert!((r.y - y).abs() < 1e-9, "{r:?}");
            assert!((r.w - w).abs() < 1e-9, "{r:?}");
            assert!((r.h - h).abs() < 1e-9, "{r:?}");
        }
    }
}
