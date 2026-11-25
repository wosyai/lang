//! Ordinary Wosy module packaged with a particular toolchain build.
//!
//! This is source transport, not an alternate implementation of checked access.

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECKED_ACCESS_SOURCE: &str = include_str!("checked_access.w");
pub const CHECKED_ACCESS_CONTRACT: &str = "D-0033/checked-access-source-v1";

/// Opaque authority for the ordinary source compiled into this release.
/// Safe hosts can request the linked component, but cannot replace its bytes,
/// package identity, contract or revision.
pub struct CheckedAccessComponent {
    _release: (),
}

impl CheckedAccessComponent {
    pub fn linked() -> Self {
        Self { _release: () }
    }

    pub fn project(&self) -> &'static str {
        "wosy-release://stdlib"
    }

    pub fn package(&self) -> &'static str {
        "wosy_toolchain_stdlib"
    }

    pub fn path(&self) -> &'static str {
        "src/checked_access.w"
    }

    pub fn text(&self) -> &'static str {
        CHECKED_ACCESS_SOURCE
    }

    pub fn revision(&self) -> &'static str {
        static REVISION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        REVISION.get_or_init(|| {
            let mut hash = blake3::Hasher::new();
            for field in [CHECKED_ACCESS_CONTRACT, PACKAGE_VERSION, CHECKED_ACCESS_SOURCE] {
                hash.update(&(field.len() as u64).to_le_bytes());
                hash.update(field.as_bytes());
            }
            hash.finalize().to_hex().to_string()
        })
    }
}
