//! The terminal service. Sprint 001 builds only the `pane-state` seed; `display`, the input verbs
//! (`key`/`mouse`/`paste`/`focus`/`resize`/`viewport`), and `create-pty` land in their own service
//! sprints (`key` is blocked on the unpinned named-key table).

pub mod cell;
pub mod dimensions;
pub mod display;
pub mod pane_state;
