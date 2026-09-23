/// An integer logical-coordinate rectangle. Empty extents are allowed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left coordinate.
    pub x: i32,
    /// Top coordinate.
    pub y: i32,
    /// Nonnegative width when normalized.
    pub width: i32,
    /// Nonnegative height when normalized.
    pub height: i32,
}

impl Rect {
    /// Constructs a rectangle, clamping negative extents and coordinate overflow.
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0).min(i32::MAX.saturating_sub(x)),
            height: height.max(0).min(i32::MAX.saturating_sub(y)),
        }
    }

    /// Normalizes rectangles supplied through public fields.
    pub fn normalized(self) -> Self {
        Self::new(self.x, self.y, self.width, self.height)
    }

    /// Exclusive right edge, with saturating arithmetic.
    pub fn right(self) -> i32 {
        self.x.saturating_add(self.width.max(0))
    }

    /// Exclusive bottom edge, with saturating arithmetic.
    pub fn bottom(self) -> i32 {
        self.y.saturating_add(self.height.max(0))
    }

    /// Whether either extent is empty.
    pub fn is_empty(self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Tests a point using half-open edges.
    pub fn contains(self, x: i32, y: i32) -> bool {
        !self.is_empty() && x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// Returns the nonempty overlap of two rectangles.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y)
            .then(|| Self::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y)))
    }

    /// Whether two rectangles have a nonempty overlap.
    pub fn intersects(self, other: Self) -> bool {
        self.intersection(other).is_some()
    }

    /// Insets evenly, reducing excessive gaps rather than inverting geometry.
    pub fn inset(self, amount: i32) -> Self {
        let area = self.normalized();
        let dx = amount.max(0).min(area.width / 2);
        let dy = amount.max(0).min(area.height / 2);
        Self::new(
            area.x + dx,
            area.y + dy,
            area.width - dx * 2,
            area.height - dy * 2,
        )
    }

    /// Fits the size and position inside bounds without mutating saved geometry.
    pub fn clamped_to(self, bounds: Self) -> Self {
        let bounds = bounds.normalized();
        let width = self.width.max(0).min(bounds.width);
        let height = self.height.max(0).min(bounds.height);
        Self::new(
            self.x.clamp(bounds.x, bounds.right() - width),
            self.y.clamp(bounds.y, bounds.bottom() - height),
            width,
            height,
        )
    }

    /// Centers without shrinking oversized extents; only coordinate overflow limits placement.
    pub fn centered_unbounded(self, width: i32, height: i32) -> Self {
        let area = self.normalized();
        let width = width.max(0);
        let height = height.max(0);
        let centered = |origin: i32, extent: i32, size: i32| {
            (i64::from(origin) + (i64::from(extent) - i64::from(size)) / 2)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX - size)) as i32
        };
        Self::new(
            centered(area.x, area.width, width),
            centered(area.y, area.height, height),
            width,
            height,
        )
    }

    /// Centers a requested size within these bounds.
    pub fn centered(self, width: i32, height: i32) -> Self {
        let area = self.normalized();
        let width = width.max(0).min(area.width);
        let height = height.max(0).min(area.height);
        Self::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    }
}
