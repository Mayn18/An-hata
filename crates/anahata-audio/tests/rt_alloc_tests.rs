//! Allocation guard: the mixer's real-time methods must never allocate or free memory.
mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anahata_audio::ActiveVoice;
use common::*;

struct CountingAlloc;

static EVENTS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

fn note() {
    // `try_with` so we stay safe during thread teardown.
    if WATCHING.try_with(|w| w.get()).unwrap_or(false) {
        EVENTS.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note();
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        note();
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        note();
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: CountingAlloc = CountingAlloc;

#[test]
fn mixer_callback_path_does_not_allocate() {
    let dir = std::env::temp_dir();
    let p1 = dir.join(format!("anahata_rt1_{}.wav", std::process::id()));
    let p2 = dir.join(format!("anahata_rt2_{}.wav", std::process::id()));
    sine_wav(&p1, 48_000, 48_000, 440.0);
    sine_wav(&p2, 48_000, 48_000, 880.0);

    let mut r = rig(&p1, 48_000, 0.0);
    let mut second = anahata_audio::Voice::spawn(2, &p2, 48_000, 0.0).unwrap();
    let consumer = second.consumer.take().unwrap();
    let second_active = ActiveVoice { id: 2, consumer, shared: Arc::clone(&second.shared) };

    // Let the workers fill their rings (allocations on other threads are not counted anyway).
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut buf = vec![0.0f32; 1024];
    let mut second_active = Some(second_active);

    EVENTS.store(0, Ordering::SeqCst);
    WATCHING.with(|w| w.set(true));
    for i in 0..300 {
        r.mixer.render(&mut buf);
        if i == 50 {
            r.mixer.set_volume(0.3);
        }
        if i == 100 {
            r.mixer.pause();
        }
        if i == 150 {
            r.mixer.resume();
        }
        if i == 200 {
            r.mixer.set_voice(second_active.take());
        }
    }
    r.mixer.stop();
    WATCHING.with(|w| w.set(false));

    assert_eq!(
        EVENTS.load(Ordering::SeqCst),
        0,
        "the audio callback path allocated or freed memory"
    );

    // The swapped-out voices were handed to the retire queue to be freed elsewhere.
    assert!(r.retired.pop().is_ok());
    let _ = std::fs::remove_file(p1);
    let _ = std::fs::remove_file(p2);
}
