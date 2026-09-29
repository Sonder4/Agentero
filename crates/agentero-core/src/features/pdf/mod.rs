//! PDF features (tauri-free subset).

pub mod layout_index;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod layout_text;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod layout_translate;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod locate;
pub mod marks;
