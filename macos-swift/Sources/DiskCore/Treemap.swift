/// A rectangle in view coordinates.
public struct Rect: Equatable, Sendable {
    public var x: Double
    public var y: Double
    public var w: Double
    public var h: Double

    public init(x: Double, y: Double, w: Double, h: Double) {
        self.x = x
        self.y = y
        self.w = w
        self.h = h
    }

    public static let zero = Rect(x: 0, y: 0, w: 0, h: 0)
}

/// Squarified treemap layout (Bruls, Huizing and van Wijk, 2000). One rect per size, in the same
/// order, tiling `bounds`. Sizes must be positive, largest first; callers drop zero sizes first.
public func layoutTreemap(_ sizes: [Double], in bounds: Rect) -> [Rect] {
    let total = sizes.reduce(0, +)
    guard total > 0, bounds.w > 0, bounds.h > 0 else {
        return Array(repeating: .zero, count: sizes.count)
    }
    let scale = bounds.w * bounds.h / total
    let areas = sizes.map { $0 * scale }
    var rects: [Rect] = []
    rects.reserveCapacity(sizes.count)
    var rest = bounds
    var start = 0
    while start < areas.count {
        let end = rowEnd(areas, start: start, short: min(rest.w, rest.h))
        let row = Array(areas[start..<end])
        if rest.w >= rest.h {
            rest = placeColumn(row, in: rest, into: &rects)
        } else {
            rest = placeStrip(row, in: rest, into: &rects)
        }
        start = end
    }
    return rects
}

/// Index one past the last area that still improves the row's worst aspect ratio.
private func rowEnd(_ areas: [Double], start: Int, short: Double) -> Int {
    var end = start + 1
    while end < areas.count && worst(areas[start...end], short: short) <= worst(areas[start..<end], short: short) {
        end += 1
    }
    return end
}

/// Lays the row as a column along the left edge and returns the remaining box.
private func placeColumn(_ row: [Double], in rest: Rect, into out: inout [Rect]) -> Rect {
    let colWidth = row.reduce(0, +) / rest.h
    var cy = rest.y
    for area in row {
        let h = area / colWidth
        out.append(Rect(x: rest.x, y: cy, w: colWidth, h: h))
        cy += h
    }
    return Rect(x: rest.x + colWidth, y: rest.y, w: rest.w - colWidth, h: rest.h)
}

/// Lays the row as a strip along the top edge and returns the remaining box.
private func placeStrip(_ row: [Double], in rest: Rect, into out: inout [Rect]) -> Rect {
    let rowHeight = row.reduce(0, +) / rest.w
    var cx = rest.x
    for area in row {
        let w = area / rowHeight
        out.append(Rect(x: cx, y: rest.y, w: w, h: rowHeight))
        cx += w
    }
    return Rect(x: rest.x, y: rest.y + rowHeight, w: rest.w, h: rest.h - rowHeight)
}

/// Worst aspect ratio of the row when it is laid along a side of length `short`.
private func worst(_ row: ArraySlice<Double>, short: Double) -> Double {
    let total = row.reduce(0, +)
    let shortSquared = short * short
    return row.map { r in
        max(shortSquared * r / (total * total), total * total / (shortSquared * r))
    }.max() ?? -.infinity
}
