use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut};

#[cfg(target_os = "kraftos")]
unsafe extern "C" {
    fn __k16_write_syscall(fd: u32, ptr: *const u8, len: u32) -> u32;
}

#[cfg(target_os = "kraftos")]
const FD_STDOUT: u32 = 1;
#[cfg(target_os = "kraftos")]
const FD_STDERR: u32 = 2;

pub struct Stdin;
pub struct Stdout;
pub type Stderr = Stdout;

impl Stdin {
    pub const fn new() -> Stdin {
        Stdin
    }
}

impl io::Read for Stdin {
    #[inline]
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }

    #[inline]
    fn read_buf(&mut self, _cursor: BorrowedCursor<'_>) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn read_vectored(&mut self, _bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        Ok(0)
    }

    #[inline]
    fn is_read_vectored(&self) -> bool {
        // Do not force `Chain<Empty, T>` or `Chain<T, Empty>` to use vectored
        // reads, unless the other reader is vectored.
        false
    }

    #[inline]
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if !buf.is_empty() { Err(io::Error::READ_EXACT_EOF) } else { Ok(()) }
    }

    #[inline]
    fn read_buf_exact(&mut self, cursor: BorrowedCursor<'_>) -> io::Result<()> {
        if cursor.capacity() != 0 { Err(io::Error::READ_EXACT_EOF) } else { Ok(()) }
    }

    #[inline]
    fn read_to_end(&mut self, _buf: &mut Vec<u8>) -> io::Result<usize> {
        Ok(0)
    }

    #[inline]
    fn read_to_string(&mut self, _buf: &mut String) -> io::Result<usize> {
        Ok(0)
    }
}

impl Stdout {
    pub const fn new() -> Stdout {
        Stdout
    }
}

impl io::Write for Stdout {
    #[inline]
    fn write(&mut self, #[cfg_attr(not(target_os = "kraftos"), allow(unused_variables))] buf: &[u8]) -> io::Result<usize> {
        #[cfg(target_os = "kraftos")]
        {
            return write_fd(FD_STDOUT, buf);
        }
        Ok(buf.len())
    }

    #[inline]
    fn write_vectored(
        &mut self,
        #[cfg_attr(not(target_os = "kraftos"), allow(unused_variables))] bufs: &[IoSlice<'_>],
    ) -> io::Result<usize> {
        #[cfg(target_os = "kraftos")]
        {
            let mut written = 0;
            for buf in bufs {
                written += write_fd(FD_STDOUT, buf)?;
            }
            return Ok(written);
        }

        let total_len = bufs.iter().map(|b| b.len()).sum();
        Ok(total_len)
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }

    #[inline]
    fn write_all(&mut self, #[cfg_attr(not(target_os = "kraftos"), allow(unused_variables))] buf: &[u8]) -> io::Result<()> {
        #[cfg(target_os = "kraftos")]
        {
            write_fd(FD_STDOUT, buf)?;
            return Ok(());
        }
        Ok(())
    }

    #[inline]
    fn write_all_vectored(
        &mut self,
        #[cfg_attr(not(target_os = "kraftos"), allow(unused_variables))] bufs: &mut [IoSlice<'_>],
    ) -> io::Result<()> {
        #[cfg(target_os = "kraftos")]
        {
            for buf in bufs {
                write_fd(FD_STDOUT, buf)?;
            }
            return Ok(());
        }

        Ok(())
    }

    // Keep the default write_fmt so the `fmt::Arguments` are still evaluated.

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "kraftos")]
fn write_fd(fd: u32, buf: &[u8]) -> io::Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let len = u32::try_from(buf.len()).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
    let written = unsafe { __k16_write_syscall(fd, buf.as_ptr(), len) };
    if written & 0x8000_0000 != 0 {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    if written != len {
        return Err(io::Error::WRITE_ALL_EOF);
    }
    Ok(buf.len())
}

pub const STDIN_BUF_SIZE: usize = 0;

pub fn is_ebadf(#[cfg_attr(target_os = "kraftos", allow(unused_variables))] _err: &io::Error) -> bool {
    #[cfg(target_os = "kraftos")]
    {
        return false;
    }

    true
}

pub fn panic_output() -> Option<Vec<u8>> {
    None
}
