//! AI Graphics: upscaling, frame generation and the files a game needs for
//! them — detected, planned, applied, verified and undone.
//!
//! This is not a performance daemon. CPU, scheduler and power policy belong to
//! falcond (see [`crate::turbo`]); this module owns what happens *inside the
//! game*: which upscaler and frame generator it uses, and any DLL or config
//! file BiGame-mode places in its folder to get there.

pub mod manifest;
pub mod optiscaler;
pub mod pe;
pub mod scan;
pub mod transaction;
