use crate::alloc::{GlobalAlloc, Layout, System};
use crate::cell::UnsafeCell;
use crate::ptr;
use crate::sync::atomic::{AtomicUsize, Ordering};

const HEAP_SIZE: usize = 256 * 1024;

struct BumpHeap(UnsafeCell<[u8; HEAP_SIZE]>);

unsafe impl Sync for BumpHeap {}

static HEAP: BumpHeap = BumpHeap(UnsafeCell::new([0; HEAP_SIZE]));
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[stable(feature = "alloc_system_type", since = "1.28.0")]
unsafe impl GlobalAlloc for System {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        if size == 0 {
            return layout.align() as *mut u8;
        }
        let align_mask = layout.align() - 1;
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let aligned = (current + align_mask) & !align_mask;
            let Some(next) = aligned.checked_add(size) else {
                return ptr::null_mut();
            };
            if next > HEAP_SIZE {
                return ptr::null_mut();
            }
            match NEXT.compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    let base = HEAP.0.get().cast::<u8>();
                    return unsafe { base.add(aligned) };
                }
                Err(updated) => current = updated,
            }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { super::realloc_fallback(self, ptr, old_layout, new_size) }
    }
}
