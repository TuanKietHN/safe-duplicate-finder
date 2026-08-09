//! Operating-system metadata and safe-move adapters.

#[cfg(not(windows))]
mod portable;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use portable::PlatformFileSystem;
#[cfg(windows)]
pub use windows::PlatformFileSystem;
