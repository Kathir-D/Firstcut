//! The command registry, app state, undo/redo, background jobs and the event stream. The UI never mutates state except by dispatching commands (§4.3). Owns the v1 Swift Pipeline port in §4.2.
