//! Port of src/window_enum.cpp.

use crate::logging::Logger;
use crate::types::{ErrorInfo, TargetWindowQuery, WindowInfo};

/// EnumWindows over all top-level windows, filling every WindowInfo field
/// (title/class as UTF-8, rects, visible/iconic/cloaked via DWM).
pub fn enumerate_windows() -> Vec<WindowInfo> {
    todo!("port EnumerateWindows")
}

/// Resolve the target window. Priority: --hwnd exact match, then
/// --foreground, then pid/title(case-insensitive substring)/class(exact)
/// filters ranked by (visible&&!iconic&&!cloaked, is-root, area) descending.
/// Returns the window and the human-readable match reason.
pub fn resolve_window_target(
    _query: &TargetWindowQuery,
    _all: &[WindowInfo],
    _logger: Option<&Logger>,
) -> Result<(WindowInfo, String), ErrorInfo> {
    todo!("port ResolveWindowTarget")
}
