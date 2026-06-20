use crate::alloc::{GlobalAlloc, Layout, System};
use crate::ptr;

unsafe extern "C" {
    fn __k16_sbrk_syscall(delta: u32) -> u32;
}

#[stable(feature = "alloc_system_type", since = "1.28.0")]
unsafe impl GlobalAlloc for System {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }
        let Some(delta) = allocation_delta(layout) else {
            return ptr::null_mut();
        };
        let old_break = unsafe { __k16_sbrk_syscall(delta) };
        if is_error_status(old_break) {
            return ptr::null_mut();
        }
        let Some(aligned) = align_up(old_break, layout.align() as u32) else {
            return ptr::null_mut();
        };
        aligned as usize as *mut u8
    }

    #[inline]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { super::realloc_fallback(self, ptr, old_layout, new_size) }
    }
}

fn allocation_delta(layout: Layout) -> Option<u32> {
    let size = u32::try_from(layout.size()).ok()?;
    let align = u32::try_from(layout.align()).ok()?;
    size.checked_add(align.checked_sub(1)?)
}

fn align_up(value: u32, alignment: u32) -> Option<u32> {
    let mask = alignment.checked_sub(1)?;
    value.checked_add(mask).map(|value| value & !mask)
}

#[inline(always)]
fn is_error_status(status: u32) -> bool {
    status & 0x8000_0000 != 0
}
