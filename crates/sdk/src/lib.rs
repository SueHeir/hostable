//! Write Hostable game rules in Rust.
//!
//! Implement [`Rules`] for a type, then call [`export!`] once in the crate.
//! Built with `cargo build --target wasm32-unknown-unknown --release` (or
//! `hostable-pack build`), the crate becomes a `rules.wasm` the host runs in
//! its sandbox; built natively, the same rules run in tests and native hosts.
//!
//! ```text
//! use hostable_sdk::prelude::*;
//!
//! #[derive(Default)]
//! pub struct MyGame;
//! impl Rules for MyGame { /* ... */ }
//!
//! hostable_sdk::export!(MyGame);
//! ```
//!
//! Rules must be deterministic: use [`Rng`] seeded from `setup`'s seed and
//! keep it in your state. Don't use `rand`, the clock or `HashMap` iteration
//! order for anything that changes the game.

pub use hostable_protocol as protocol;
pub use hostable_rules_api as rules_api;
pub use hostable_rules_api::abi;
pub use serde_json;

pub mod prelude {
    pub use hostable_protocol::{
        ButtonStyle, CardFace, ChoiceOption, ChoiceStyle, Direction, Feedback, MeterStyle,
        PhoneInput, PlayerPanel, Scene, Shape, Tab, TableView, TextStyle, Widget,
    };
    pub use hostable_rules_api::{
        rank_by_score, Event, OnTimeout, Outcome, Results, Rng, Rules, RulesError, RulesResult,
        Seat, SeatId, Wait,
    };
    pub use serde::{Deserialize, Serialize};
    pub use serde_json::{json, Value};
}

pub use prelude::*;

/// Send a line to the host's log (only works inside the sandbox; natively it
/// prints to stderr).
pub fn log(message: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        #[link(wasm_import_module = "env")]
        extern "C" {
            #[link_name = "log"]
            fn host_log(ptr: *const u8, len: usize);
        }
        // SAFETY: the host only reads `len` bytes from `ptr` during the call.
        unsafe { host_log(message.as_ptr(), message.len()) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[rules] {message}");
}

/// The module-side half of the wasm ABI. Used by [`export!`].
#[doc(hidden)]
pub mod __abi {
    use std::cell::RefCell;

    thread_local! {
        static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    pub fn alloc(len: usize) -> *mut u8 {
        let mut buf = Vec::<u8>::with_capacity(len.max(1));
        let ptr = buf.as_mut_ptr();
        std::mem::forget(buf);
        ptr
    }

    /// # Safety
    /// `ptr` and `len` must come from [`alloc`].
    pub unsafe fn free(ptr: *mut u8, len: usize) {
        drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
    }

    /// # Safety
    /// `ptr..ptr+len` must be a buffer from [`alloc`] holding the request.
    pub unsafe fn call<R: crate::Rules>(rules: &R, ptr: *const u8, len: usize) -> u64 {
        let request = std::slice::from_raw_parts(ptr, len);
        let response = crate::abi::dispatch(rules, request);
        OUT.with(|out| {
            let mut out = out.borrow_mut();
            *out = response;
            ((out.as_ptr() as u64) << 32) | out.len() as u64
        })
    }
}

/// Export a [`Rules`] implementation (a type with `Default`) as the wasm ABI.
/// Does nothing when not compiling to wasm32.
#[macro_export]
macro_rules! export {
    ($rules:ty) => {
        #[cfg(target_arch = "wasm32")]
        mod __hostable_export {
            use super::*;

            #[no_mangle]
            pub extern "C" fn hostable_abi_version() -> i32 {
                $crate::abi::ABI_VERSION
            }

            #[no_mangle]
            pub extern "C" fn hostable_alloc(len: usize) -> *mut u8 {
                $crate::__abi::alloc(len)
            }

            /// # Safety
            /// Called by the host with a buffer from `hostable_alloc`.
            #[no_mangle]
            pub unsafe extern "C" fn hostable_free(ptr: *mut u8, len: usize) {
                $crate::__abi::free(ptr, len)
            }

            /// # Safety
            /// Called by the host with a request it wrote into `hostable_alloc`.
            #[no_mangle]
            pub unsafe extern "C" fn hostable_call(ptr: *const u8, len: usize) -> u64 {
                let rules = <$rules as ::core::default::Default>::default();
                $crate::__abi::call(&rules, ptr, len)
            }
        }
    };
}
