//! Scroll-offset helper shared by every list/table/text view.

/// A new scroll `top` (index of the first visible row/column) that keeps
/// `cursor` inside a viewport of `height` rows: it scrolls up to `cursor` when
/// the cursor sits above `top`, and down just far enough when it sits below the
/// last visible row. A zero `height` leaves `top` unchanged.
///
/// This is the one place the "keep the selection visible" idiom lives; callers
/// used to open-code `if cursor < top { top = cursor } else if cursor >= top +
/// height { top = cursor + 1 - height }` in ~20 views.
pub fn scroll_to_visible(top: usize, cursor: usize, height: usize) -> usize {
    if cursor < top {
        cursor
    } else if height > 0 && cursor >= top + height {
        cursor + 1 - height
    } else {
        top
    }
}

#[cfg(test)]
mod tests {
    use super::scroll_to_visible;

    #[test]
    fn keeps_cursor_within_the_window() {
        // Cursor already visible: unchanged.
        assert_eq!(scroll_to_visible(0, 3, 10), 0);
        assert_eq!(scroll_to_visible(5, 9, 10), 5, "cursor at top+height-1 stays");
        // Cursor above the window: scroll up to it.
        assert_eq!(scroll_to_visible(5, 2, 10), 2);
        // Cursor below the window: scroll down just enough to show it last.
        assert_eq!(scroll_to_visible(0, 10, 10), 1);
        assert_eq!(scroll_to_visible(0, 12, 10), 3);
        // Computing the first-visible from a zero baseline.
        assert_eq!(scroll_to_visible(0, 4, 5), 0);
        assert_eq!(scroll_to_visible(0, 5, 5), 1);
    }

    #[test]
    fn zero_height_leaves_top_untouched() {
        assert_eq!(scroll_to_visible(7, 40, 0), 7);
    }
}
