//! Vard reference runtime.
//!
//! Deterministic and headless by design: the same reactive core, layout and inspection
//! run on a developer machine in milliseconds (for AI-driven iteration and tests) and
//! behind a platform shell on devices.

pub mod actions;
mod eval;
pub mod inspect;
pub mod json;
pub mod layout;
pub mod native;
pub mod node;
pub mod raster;
pub mod rt;
pub mod text;
pub mod value;

pub use actions::ACTIONS_HELP;
pub use rt::Runtime;

/// `vard explain` entries for runtime (R) and lint (L) codes.
pub const EXPLAIN: &[(&str, &str)] = &[
    ("R0001", "A value was missing at runtime: list index out of range, `!` on nil, or an assignment into an element that no longer exists. Guard with `.count`, `?? fallback` or `!= nil`."),
    ("R0002", "Two items of a keyed `for` produced the same `by` key. Keys identify rows; make them unique (usually a stable id field, not the title or the index)."),
    ("R0003", "State was written while a prop, `let` or `if`/`for` condition was being evaluated. Rendering must be pure. Move the write into an event handler (onTap, onChange, onSubmit) or a fn called from one."),
    ("R0004", "A `let` (or parameter expression) reads itself, directly or through other derived values."),
    ("R0005", "A function recursed more than 200 levels deep."),
    ("R0006", "Effects kept invalidating each other. Usually caused by R0003-style writes during rendering."),
    ("R0009", "An in-place change targeted something that is not a mutable variable."),
    ("R0010", "An `extern fn` has no native implementation on this device. Implement it in a native module (`vard bindings --kotlin` generates the typed base class; register it in `VardActivity.modules()` or via manifest meta-data `dev.vard.module.<name>`), or add the module library that provides it (e.g. -Pvard.firebase=true)."),
    ("R0011", "A native value did not match the declared enum (e.g. the host answered \"Granted\" for PermissionStatus). The first variant was used. Fix the native side or the enum."),
    ("L001", "Children extend past their container. Vertically: put the content in `Scroll(grow: 1) { ... }`. Horizontally: give the text/flexible child `grow: 1` so it takes only the remaining space and wraps, or reduce fixed sizes."),
    ("L002", "Interactive elements need at least 44x44dp to be reliably tappable. Add `height: 44`, padding, or use the std `Button`."),
    ("L004", "A container with children has zero width or height. `why @id` shows which rule produced the size (often a missing `grow: 1` or a zero-sized parent)."),
    ("L005", "Text/background contrast is below WCAG AA (4.5:1 body text, 3:1 large text). Use `.text`/`.muted` on `.background`/`.surface`, `.onPrimary` on `.primary`, or adjust your theme tokens."),
    ("L006", "`id:` values must be unique on screen; they are used as test and inspection handles (`#id`)."),
];
