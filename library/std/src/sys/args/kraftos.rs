//! K16/KraftOS command-line arguments.
//!
//! KraftOS child entry uses `r1 = argc` and `r2 = argv_table`. Each table entry
//! is two little-endian `u32` words: byte pointer and byte length. Arguments are
//! not C strings and are not null-terminated.

pub use super::common::Args;
use crate::ffi::OsString;
use crate::sync::atomic::{Atomic, AtomicIsize, AtomicPtr, Ordering};
use crate::{ptr, slice};

const ARG_ENTRY_BYTES: usize = 8;
const MAX_KRAFTOS_ARGS: isize = 4;

static ARGC: Atomic<isize> = AtomicIsize::new(0);
static ARGV: Atomic<*mut u8> = AtomicPtr::new(ptr::null_mut());

pub unsafe fn init(argc: isize, argv: *const *const u8) {
    ARGC.store(argc, Ordering::Relaxed);
    ARGV.store(argv.cast_mut().cast(), Ordering::Relaxed);
}

pub fn args() -> Args {
    let argc = ARGC.load(Ordering::Relaxed);
    let table = ARGV.load(Ordering::Relaxed);
    if argc < 0 || argc > MAX_KRAFTOS_ARGS {
        panic!("invalid KraftOS argc");
    }
    if argc == 0 {
        return Args::new(Vec::new());
    }
    if table.is_null() {
        panic!("invalid KraftOS argv table");
    }

    let mut args = Vec::with_capacity(argc as usize);
    for index in 0..argc as usize {
        let entry = unsafe { table.add(index * ARG_ENTRY_BYTES) };
        let ptr = unsafe { read_u32(entry) };
        let len = unsafe { read_u32(entry.add(4)) };
        if ptr == 0 && len != 0 {
            panic!("invalid KraftOS argv entry");
        }
        let bytes = if len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(ptr as *const u8, len as usize) }
        };
        args.push(unsafe { OsString::from_encoded_bytes_unchecked(bytes.to_vec()) });
    }

    Args::new(args)
}

unsafe fn read_u32(ptr: *const u8) -> u32 {
    unsafe { core::ptr::read_unaligned(ptr.cast()) }
}
