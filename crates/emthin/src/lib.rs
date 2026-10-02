pub mod activation;
pub mod cli;
pub mod clipboard_bridge;
pub mod doc_render;
pub mod docui;
pub mod element;
pub mod figure_render;
pub mod grabs;
pub mod handlers;
pub mod input;
pub mod ipc;
pub mod mirror_render;
pub mod protocols;
pub mod session;
pub mod state;
pub mod tick;
pub mod util;
pub mod winit;
pub mod xwayland_satellite;

// Re-export state sub-modules at crate root so `crate::apps::*`,
// `crate::focus::*`, `crate::ime::*`, `crate::page::*` paths resolve
// without qualifying through `crate::state`.
pub use state::{apps, cursor, focus, host, ime, page, xwayland};
pub use state::{EmthinState, KeyboardFocusTarget};
