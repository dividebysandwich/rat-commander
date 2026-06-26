//! Panel split orientation.

/// How the two panels are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// Side by side (the classic two-pane look).
    Vertical,
    /// Stacked one above the other.
    Horizontal,
}

impl SplitDir {
    pub fn toggle(self) -> Self {
        match self {
            SplitDir::Vertical => SplitDir::Horizontal,
            SplitDir::Horizontal => SplitDir::Vertical,
        }
    }
}
