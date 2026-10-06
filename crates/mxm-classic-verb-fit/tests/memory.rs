//! **A response the machine cannot hold is refused by name, never an abort.**
//!
//! The fit runs inside a plugin, on the host's background thread, and an allocation that fails the
//! ordinary way aborts the process — the host with it. So this binary's allocator stands in for a
//! machine that has run out: it refuses any single allocation over [`LIMIT`]. A test file is its own
//! binary, so nothing else sees it, and the file holds one test so nothing runs beside it.

use std::alloc::{GlobalAlloc, Layout, System};

use mxm_classic_verb_fit::{FitRefusal, Refusal, analyse, fit};

/// Room for the response this test holds — thirty seconds at 192 kHz, the input domain's ceiling, is
/// 22 MB of `f32` — and none for one `f64` copy of it, 44 MB.
const LIMIT: usize = 32 << 20;

struct Scarce;

// SAFETY: every call is forwarded to the system allocator unchanged, or answered with null, which the
// `GlobalAlloc` contract allows for any allocation it cannot satisfy.
unsafe impl GlobalAlloc for Scarce {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > LIMIT {
            return core::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > LIMIT {
            return core::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > LIMIT {
            return core::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static SCARCE: Scarce = Scarce;

#[test]
fn a_response_too_large_for_memory_is_refused_by_name_not_aborted() {
    let rate = 192_000u32;
    let frames = 30 * rate as usize;
    // A direct sound early and a last sample that is not zero, so the whole length is the response's.
    let mut response = vec![0.0f32; frames];
    response[rate as usize / 20] = 1.0;
    response[frames - 1] = 1.0e-3;
    let channels = [response.as_slice()];
    // The first buffer the analysis takes is the combined energy, one `f64` per frame.
    let energy = frames * core::mem::size_of::<f64>();

    assert_eq!(
        analyse(&channels, rate as f32).err(),
        Some(Refusal::OutOfMemory { bytes: energy })
    );
    let refused = fit(&channels, rate as f32).err();
    assert_eq!(refused, Some(FitRefusal::OutOfMemory { bytes: energy }));
    assert!(
        refused.is_some_and(|refusal| refusal
            .to_string()
            .contains("43.9 MB could not be reserved")),
        "the refusal does not say what it could not have"
    );
}
