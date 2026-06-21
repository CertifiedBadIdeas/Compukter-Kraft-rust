use crate::ffi::OsString;
use crate::fmt;
use crate::fs::TryLockError;
use crate::hash::{Hash, Hasher};
use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut, SeekFrom};
use crate::path::{Path, PathBuf};
pub use crate::sys::fs::common::Dir;
use crate::sys::time::SystemTime;
use crate::sys::unsupported;
use crate::vec::Vec;
use core::cell::Cell;

unsafe extern "C" {
    fn __k16_open_syscall(ptr: *const u8, len: u32, flags: u32) -> u32;
    fn __k16_read_syscall(fd: u32, ptr: *mut u8, len: u32) -> u32;
    fn __k16_write_syscall(fd: u32, ptr: *const u8, len: u32) -> u32;
    fn __k16_read_dir_syscall(ptr: *const u8, len: u32) -> u32;
    fn __k16_stat_syscall(ptr: *const u8, len: u32, out: *mut u8) -> u32;
    fn __k16_close_syscall(fd: u32) -> u32;
}

const READ_DIR_REQUEST_MAGIC: u32 = 0x5249_4452;
const MAX_READ_DIR_PATH_BYTES: usize = 228;
const MAX_READ_DIR_REQUEST_BYTES: usize = 16 + MAX_READ_DIR_PATH_BYTES;
const STAT_METADATA_BYTES: usize = 16;
const FILE_TYPE_REGULAR: u32 = 1;
const FILE_TYPE_DIRECTORY: u32 = 2;
const FILE_ATTR_DIRECTORY_BIT: u32 = 0x0100_0000;
const OPEN_READ_ONLY: u32 = 0;
const OPEN_WRITE_ONLY: u32 = 1;
const OPEN_CREATE: u32 = 1 << 1;
const OPEN_TRUNCATE: u32 = 1 << 2;
const OPEN_APPEND: u32 = 1 << 3;
const READ_BOUNCE_SIZE: usize = 512;
const READ_DIR_BOUNCE_SIZE: usize = 4096;

// KraftOS userland is currently single-threaded. Keep kernel writes away from
// caller stack probes until the K16 backend's cross-ABI stack writes are solid.
static mut READ_BOUNCE: [u8; READ_BOUNCE_SIZE] = [0; READ_BOUNCE_SIZE];
static mut READ_DIR_BOUNCE: [u8; READ_DIR_BOUNCE_SIZE] = [0; READ_DIR_BOUNCE_SIZE];
static mut STAT_BOUNCE: [u8; STAT_METADATA_BYTES] = [0; STAT_METADATA_BYTES];

pub struct File {
    fd: u32,
    eof: Cell<bool>,
}

pub struct FileAttr {
    packed: u32,
}

pub struct ReadDir {
    entries: Vec<DirEntry>,
    index: usize,
}

#[derive(Clone)]
pub struct DirEntry {
    path: PathBuf,
    name: OsString,
    file_type: FileType,
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct FileTimes {}

pub struct FilePermissions {
    readonly: bool,
}

pub struct FileType {
    kind: u32,
}

#[derive(Debug)]
pub struct DirBuilder {}

impl FileAttr {
    pub fn size(&self) -> u64 {
        let size = if self.packed >= FILE_ATTR_DIRECTORY_BIT {
            self.packed - FILE_ATTR_DIRECTORY_BIT
        } else {
            self.packed
        };
        u64::from(size)
    }

    pub fn perm(&self) -> FilePermissions {
        FilePermissions { readonly: false }
    }

    pub fn file_type(&self) -> FileType {
        let kind = if self.packed >= FILE_ATTR_DIRECTORY_BIT {
            FILE_TYPE_DIRECTORY
        } else {
            FILE_TYPE_REGULAR
        };
        FileType { kind }
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        unsupported()
    }
}

impl Clone for FileAttr {
    fn clone(&self) -> FileAttr {
        FileAttr { packed: self.packed }
    }
}

impl FilePermissions {
    pub fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }
}

impl Clone for FilePermissions {
    fn clone(&self) -> FilePermissions {
        FilePermissions { readonly: self.readonly }
    }
}

impl PartialEq for FilePermissions {
    fn eq(&self, other: &FilePermissions) -> bool {
        self.readonly == other.readonly
    }
}

impl Eq for FilePermissions {}

impl fmt::Debug for FilePermissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilePermissions").field("readonly", &self.readonly).finish()
    }
}

impl FileTimes {
    pub fn set_accessed(&mut self, _t: SystemTime) {}
    pub fn set_modified(&mut self, _t: SystemTime) {}
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.kind == FILE_TYPE_DIRECTORY
    }

    pub fn is_file(&self) -> bool {
        self.kind == FILE_TYPE_REGULAR
    }

    pub fn is_symlink(&self) -> bool {
        false
    }
}

impl Clone for FileType {
    fn clone(&self) -> FileType {
        *self
    }
}

impl Copy for FileType {}

impl PartialEq for FileType {
    fn eq(&self, other: &FileType) -> bool {
        self.kind == other.kind
    }
}

impl Eq for FileType {}

impl Hash for FileType {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.kind.hash(h);
    }
}

impl fmt::Debug for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileType").field("kind", &self.kind).finish()
    }
}

impl fmt::Debug for ReadDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadDir")
            .field("len", &self.entries.len())
            .field("index", &self.index)
            .finish()
    }
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        let entry = self.entries.get(self.index)?.clone();
        self.index += 1;
        Some(Ok(entry))
    }
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn file_name(&self) -> OsString {
        self.name.clone()
    }

    pub fn metadata(&self) -> io::Result<FileAttr> {
        stat(&self.path)
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        Ok(self.file_type)
    }
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    pub fn read(&mut self, read: bool) {
        self.read = read;
    }

    pub fn write(&mut self, write: bool) {
        self.write = write;
    }

    pub fn append(&mut self, append: bool) {
        self.append = append;
    }

    pub fn truncate(&mut self, truncate: bool) {
        self.truncate = truncate;
    }

    pub fn create(&mut self, create: bool) {
        self.create = create;
    }

    pub fn create_new(&mut self, create_new: bool) {
        self.create_new = create_new;
    }
}

impl File {
    pub fn open(path: &Path, opts: &OpenOptions) -> io::Result<File> {
        if opts.create_new || opts.read == opts.write && !opts.append {
            return unsupported();
        }
        let flags = if opts.read {
            if opts.append || opts.truncate || opts.create {
                return unsupported();
            }
            OPEN_READ_ONLY
        } else {
            let mut flags = OPEN_WRITE_ONLY;
            if opts.create {
                flags |= OPEN_CREATE;
            }
            if opts.truncate {
                flags |= OPEN_TRUNCATE;
            }
            if opts.append {
                flags |= OPEN_APPEND;
            }
            flags
        };

        let path = path.as_os_str().as_encoded_bytes();
        let len = u32::try_from(path.len()).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
        let fd = unsafe { __k16_open_syscall(path.as_ptr(), len, flags) };
        if syscall_failed(fd) {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        Ok(File { fd, eof: Cell::new(false) })
    }

    pub fn file_attr(&self) -> io::Result<FileAttr> {
        unsupported()
    }

    pub fn fsync(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn datasync(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn lock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn lock_shared(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn try_lock(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(io::Error::UNSUPPORTED_PLATFORM))
    }

    pub fn try_lock_shared(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(io::Error::UNSUPPORTED_PLATFORM))
    }

    pub fn unlock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn truncate(&self, _size: u64) -> io::Result<()> {
        unsupported()
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.eof.get() {
            return Ok(0);
        }
        let request = core::cmp::min(buf.len(), READ_BOUNCE_SIZE);
        let len = u32::try_from(request).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
        let bounce = core::ptr::addr_of_mut!(READ_BOUNCE).cast::<u8>();
        let read = unsafe { __k16_read_syscall(self.fd, bounce, len) };
        if syscall_failed(read) {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        let read = usize::try_from(read).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
        if read > request {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        if read < request {
            self.eof.set(true);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(bounce.cast_const(), buf.as_mut_ptr(), read);
        }
        Ok(read)
    }

    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        crate::io::default_read_vectored(|buf| self.read(buf), bufs)
    }

    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn read_buf(&self, cursor: BorrowedCursor<'_>) -> io::Result<()> {
        crate::io::default_read_buf(|buf| self.read(buf), cursor)
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let len = u32::try_from(buf.len()).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
        let written = unsafe { __k16_write_syscall(self.fd, buf.as_ptr(), len) };
        if syscall_failed(written) {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        if written > len {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        usize::try_from(written).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)
    }

    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        crate::io::default_write_vectored(|buf| self.write(buf), bufs)
    }

    pub fn is_write_vectored(&self) -> bool {
        false
    }

    pub fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn seek(&self, _pos: SeekFrom) -> io::Result<u64> {
        unsupported()
    }

    pub fn size(&self) -> Option<io::Result<u64>> {
        None
    }

    pub fn tell(&self) -> io::Result<u64> {
        unsupported()
    }

    pub fn duplicate(&self) -> io::Result<File> {
        unsupported()
    }

    pub fn set_permissions(&self, _perm: FilePermissions) -> io::Result<()> {
        unsupported()
    }

    pub fn set_times(&self, _times: FileTimes) -> io::Result<()> {
        unsupported()
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = unsafe { __k16_close_syscall(self.fd) };
    }
}

impl DirBuilder {
    pub fn new() -> DirBuilder {
        DirBuilder {}
    }

    pub fn mkdir(&self, _p: &Path) -> io::Result<()> {
        unsupported()
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File").field("fd", &self.fd).finish()
    }
}

pub fn readdir(p: &Path) -> io::Result<ReadDir> {
    let path = p.as_os_str().as_encoded_bytes();
    if path.is_empty() || path.len() > MAX_READ_DIR_PATH_BYTES {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }

    let mut request = [0u8; MAX_READ_DIR_REQUEST_BYTES];
    write_u32_le(&mut request, 0, READ_DIR_REQUEST_MAGIC);
    write_u32_le(&mut request, 4, path.len() as u32);
    let out = core::ptr::addr_of_mut!(READ_DIR_BOUNCE).cast::<u8>();
    write_u32_le(&mut request, 8, out as usize as u32);
    write_u32_le(&mut request, 12, READ_DIR_BOUNCE_SIZE as u32);
    request[16..16 + path.len()].copy_from_slice(path);

    let request_len =
        u32::try_from(16 + path.len()).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
    let read = unsafe { __k16_read_dir_syscall(request.as_ptr(), request_len) };
    if syscall_failed(read) {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    let read = usize::try_from(read).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
    if read > READ_DIR_BOUNCE_SIZE {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }

    let listing = unsafe { core::slice::from_raw_parts(out.cast_const(), read) };
    let mut entries = Vec::new();
    let mut cursor = 0;
    while cursor < listing.len() {
        let start = cursor;
        while cursor < listing.len() && listing[cursor] != b'\n' {
            cursor += 1;
        }
        let name_bytes = &listing[start..cursor];
        if name_bytes.is_empty() {
            return Err(io::Error::UNSUPPORTED_PLATFORM);
        }
        let name = unsafe { OsString::from_encoded_bytes_unchecked(name_bytes.to_vec()) };
        let path = child_path(p, &name)?;
        let file_type = stat(&path)?.file_type();
        entries.push(DirEntry { path, name, file_type });
        if cursor < listing.len() {
            cursor += 1;
        }
    }

    Ok(ReadDir { entries, index: 0 })
}

pub fn unlink(_p: &Path) -> io::Result<()> {
    unsupported()
}

pub fn rename(_old: &Path, _new: &Path) -> io::Result<()> {
    unsupported()
}

pub fn set_perm(_p: &Path, _perm: FilePermissions) -> io::Result<()> {
    unsupported()
}

pub fn set_times(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn set_times_nofollow(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn rmdir(_p: &Path) -> io::Result<()> {
    unsupported()
}

pub fn remove_dir_all(_path: &Path) -> io::Result<()> {
    unsupported()
}

pub fn exists(_path: &Path) -> io::Result<bool> {
    unsupported()
}

pub fn readlink(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn symlink(_original: &Path, _link: &Path) -> io::Result<()> {
    unsupported()
}

pub fn link(_src: &Path, _dst: &Path) -> io::Result<()> {
    unsupported()
}

pub fn stat(p: &Path) -> io::Result<FileAttr> {
    let path = p.as_os_str().as_encoded_bytes();
    if path.is_empty() || path.len() > MAX_READ_DIR_PATH_BYTES {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    let len = u32::try_from(path.len()).map_err(|_| io::Error::UNSUPPORTED_PLATFORM)?;
    let out = core::ptr::addr_of_mut!(STAT_BOUNCE).cast::<u8>();
    let status = unsafe { __k16_stat_syscall(path.as_ptr(), len, out) };
    if syscall_failed(status) {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    let metadata = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(STAT_BOUNCE)) };
    let kind = read_u32_le(&metadata, 0);
    if kind != FILE_TYPE_REGULAR && kind != FILE_TYPE_DIRECTORY {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    let size = read_u32_le(&metadata, 4);
    if size >= FILE_ATTR_DIRECTORY_BIT {
        return Err(io::Error::UNSUPPORTED_PLATFORM);
    }
    let packed = if kind == FILE_TYPE_DIRECTORY { size + FILE_ATTR_DIRECTORY_BIT } else { size };
    Ok(FileAttr { packed })
}

pub fn lstat(p: &Path) -> io::Result<FileAttr> {
    stat(p)
}

pub fn canonicalize(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn copy(_from: &Path, _to: &Path) -> io::Result<u64> {
    unsupported()
}

fn syscall_failed(status: u32) -> bool {
    status & 0x8000_0000 != 0
}

fn child_path(parent: &Path, name: &OsString) -> io::Result<PathBuf> {
    let mut path = parent.to_path_buf();
    path.push(Path::new(name));
    Ok(path)
}

fn write_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}
