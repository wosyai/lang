//! Ordinary Wosy module packaged with a particular toolchain build.
//!
//! This is source transport, not an alternate implementation of checked access.

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECKED_ACCESS_SOURCE: &str = include_str!("checked_access.w");
pub const CHECKED_ACCESS_CONTRACT: &str = "D-0033/checked-access-source-v1";
