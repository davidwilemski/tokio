//! Asynchronous synchronization primitives.
//!
//! This crate provides primitives for synchronizing asynchronous tasks.

extern crate futures;

macro_rules! debug {
    ($($t:tt)*) => {
        if false {
            println!($($t)*);
        }
    }
}

macro_rules! if_fuzz {
    ($($t:tt)*) => {{
        if false { $($t)* }
    }}
}

mod loom;
pub mod oneshot;
pub mod mpsc;
pub mod semaphore;
pub mod task;
