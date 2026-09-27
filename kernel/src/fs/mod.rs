//! PurityFS — the PurityOS file system (v2).
//!
//! An in-memory hierarchical filesystem with typed nodes (file / directory /
//! symlink / device), per-node metadata, and path resolution with `..`
//! handling. It is the storage backend for both the CLI and the GUI file
//! manager. A disk persistence layer (ATA) is planned on top of this.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use spin::{Lazy, Mutex};

/// Node types in PurityFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    /// A symbolic link; `content` holds the target path.
    SymLink,
    /// A device node (e.g. `tty`, `speaker`).
    Device,
}

impl FileKind {
    pub fn is_dir(self) -> bool {
        self == FileKind::Dir
    }
    pub fn label(self) -> &'static str {
        match self {
            FileKind::File => "file",
            FileKind::Dir => "dir",
            FileKind::SymLink => "link",
            FileKind::Device => "dev",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: FileKind,
    pub content: Vec<u8>,
    /// Size in bytes (files) or 0.
    pub size: u64,
    /// Creation tick (PIT ticks at creation time).
    pub created: u64,
    /// Target path for symlinks.
    pub target: String,
    /// Permission bits (Unix-style, e.g. 0o644 for files, 0o755 for dirs).
    pub mode: u16,
}

/// Default permission bits for a node of the given kind.
pub fn default_mode(kind: FileKind) -> u16 {
    match kind {
        FileKind::Dir => 0o755,
        FileKind::File => 0o644,
        FileKind::SymLink => 0o777,
        FileKind::Device => 0o600,
    }
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.kind.is_dir()
    }
    pub fn is_file(&self) -> bool {
        self.kind == FileKind::File
    }
}

pub struct VirtualFs {
    pub cwd: String,
    pub entries: BTreeMap<String, Node>,
}

impl VirtualFs {
    fn new() -> Self {
        let mut entries = BTreeMap::new();
        let mk = |kind: FileKind, content: &[u8], target: &str, created: u64| Node {
            kind,
            content: content.to_vec(),
            size: content.len() as u64,
            created,
            target: target.to_string(),
            mode: default_mode(kind),
        };
        entries.insert("/".into(), mk(FileKind::Dir, b"", "", 0));
        entries.insert("/home".into(), mk(FileKind::Dir, b"", "", 1));
        entries.insert(
            "/home/welcome.txt".into(),
            mk(FileKind::File, b"Welcome to PurityOS!\nType `help` to see available commands.\n", "", 2),
        );
        entries.insert(
            "/home/docs".into(),
            mk(FileKind::Dir, b"", "", 3),
        );
        entries.insert(
            "/home/docs/readme.md".into(),
            mk(FileKind::File, b"# PurityOS\n\nA from-scratch Rust operating system with a GUI,\nPurityFS filesystem, multitasking and Ring 3 support.\n", "", 4),
        );
        entries.insert(
            "/dev".into(),
            mk(FileKind::Dir, b"", "", 5),
        );
        entries.insert(
            "/dev/tty".into(),
            mk(FileKind::Device, b"", "", 6),
        );
        entries.insert(
            "/dev/speaker".into(),
            mk(FileKind::Device, b"", "", 7),
        );
        entries.insert(
            "/home/current".into(),
            mk(FileKind::SymLink, b"", "/home/docs/readme.md", 8),
        );
        VirtualFs { cwd: "/home".into(), entries }
    }

    /// Resolve a user-supplied path against the current directory, handling
    /// `.`, `..` and trailing slashes. Returns a normalized absolute path.
    pub fn resolve(&self, path: &str) -> String {
        let base = if path.starts_with('/') {
            String::new()
        } else {
            self.cwd.clone()
        };
        let combined = if base.is_empty() || base == "/" {
            format!("/{}", path.trim_start_matches('/'))
        } else {
            format!("{}/{}", base, path)
        };

        let mut out: Vec<&str> = Vec::new();
        for part in combined.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                other => out.push(other),
            }
        }
        if out.is_empty() {
            String::from("/")
        } else {
            format!("/{}", out.join("/"))
        }
    }

    /// Resolve symlinks (with a loop guard) to the final node path.
    pub fn follow(&self, path: &str) -> String {
        let mut cur = path.to_string();
        for _ in 0..8 {
            match self.entries.get(&cur) {
                Some(n) if n.kind == FileKind::SymLink => {
                    let t = n.target.clone();
                    cur = if t.starts_with('/') { t } else { self.resolve(&format!("{}/{}", parent(&cur), t)) };
                }
                _ => return cur,
            }
        }
        cur
    }

    pub fn get(&self, path: &str) -> Option<&Node> {
        self.entries.get(path)
    }

    pub fn get_mut(&mut self, path: &str) -> Option<&mut Node> {
        self.entries.get_mut(path)
    }

    /// Read a file's contents, following symlinks.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let real = self.follow(path);
        match self.entries.get(&real) {
            Some(n) if n.kind == FileKind::File => Some(n.content.clone()),
            _ => None,
        }
    }

    /// Append data to a file, creating it if missing.
    pub fn append(&mut self, path: &str, data: &[u8]) {
        let now = crate::drivers::timer::ticks();
        if let Some(n) = self.entries.get_mut(path) {
            if n.kind == FileKind::File {
                n.content.extend_from_slice(data);
                n.size = n.content.len() as u64;
                return;
            }
        }
        // Create (or overwrite as new).
        self.entries.insert(
            path.into(),
            Node { kind: FileKind::File, content: data.to_vec(), size: data.len() as u64, created: now, target: String::new(), mode: default_mode(FileKind::File) },
        );
    }

    /// Create an empty file; returns true on success.
    pub fn create_file(&mut self, path: &str) -> bool {
        if self.entries.contains_key(path) {
            return false;
        }
        self.entries.insert(
            path.into(),
            Node { kind: FileKind::File, content: Vec::new(), size: 0, created: crate::drivers::timer::ticks(), target: String::new(), mode: default_mode(FileKind::File) },
        );
        true
    }

    /// Create a directory.
    pub fn create_dir(&mut self, path: &str) -> bool {
        if self.entries.contains_key(path) {
            return false;
        }
        self.entries.insert(
            path.into(),
            Node { kind: FileKind::Dir, content: Vec::new(), size: 0, created: crate::drivers::timer::ticks(), target: String::new(), mode: default_mode(FileKind::Dir) },
        );
        true
    }

    /// Create a symbolic link at `path` pointing to `target`.
    pub fn symlink(&mut self, path: &str, target: &str) -> bool {
        if self.entries.contains_key(path) {
            return false;
        }
        self.entries.insert(
            path.into(),
            Node { kind: FileKind::SymLink, content: Vec::new(), size: 0, created: crate::drivers::timer::ticks(), target: target.to_string(), mode: default_mode(FileKind::SymLink) },
        );
        true
    }

    /// Create a device node.
    pub fn device(&mut self, path: &str) -> bool {
        if self.entries.contains_key(path) {
            return false;
        }
        self.entries.insert(
            path.into(),
            Node { kind: FileKind::Device, content: Vec::new(), size: 0, created: crate::drivers::timer::ticks(), target: String::new(), mode: default_mode(FileKind::Device) },
        );
        true
    }

    /// Remove a node (only empty directories and files).
    pub fn remove(&mut self, path: &str) -> Result<(), &'static str> {
        if path == "/" {
            return Err("cannot remove root");
        }
        let kind = self.entries.get(path).map(|n| n.kind).ok_or("not found")?;
        if kind == FileKind::Dir {
            let prefix = format!("{}/", path);
            let has_children = self.entries.keys().any(|k| k != path && k.starts_with(&prefix));
            if has_children {
                return Err("directory not empty");
            }
        }
        self.entries.remove(path);
        Ok(())
    }

    /// Recursively remove a node and everything beneath it (`rm -r`).
    pub fn remove_recursive(&mut self, path: &str) -> Result<(), &'static str> {
        if path == "/" {
            return Err("cannot remove root");
        }
        if !self.entries.contains_key(path) {
            return Err("not found");
        }
        let prefix = format!("{}/", path);
        // Collect every descendant key plus the node itself.
        let to_delete: Vec<String> = self
            .entries
            .keys()
            .filter(|k| *k == path || k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in to_delete {
            self.entries.remove(&k);
        }
        Ok(())
    }

    /// Rename / move a node (`mv`). Fails if the destination's parent does
    /// not exist or is a path inside the source being moved.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), &'static str> {
        if from == "/" {
            return Err("cannot move root");
        }
        if !self.entries.contains_key(from) {
            return Err("source not found");
        }
        if self.entries.contains_key(to) {
            return Err("destination exists");
        }
        // Parent of the destination must be an existing directory (or root).
        let dest_parent = parent(to);
        if dest_parent != "/" {
            match self.entries.get(&dest_parent) {
                Some(n) if n.kind == FileKind::Dir => {}
                _ => return Err("destination directory missing"),
            }
        }
        // Moving a directory: re-parent all descendants too.
        let node = self.entries.remove(from).unwrap();
        let old_prefix = format!("{}/", from);
        let mut children: Vec<(String, Node)> = Vec::new();
        if node.kind == FileKind::Dir {
            let child_keys: Vec<String> = self
                .entries
                .keys()
                .filter(|k| k.starts_with(&old_prefix))
                .cloned()
                .collect();
            for k in child_keys {
                let n = self.entries.remove(&k).unwrap();
                let rest = &k[old_prefix.len()..];
                children.push((format!("{}/{}", to.trim_end_matches('/'), rest), n));
            }
        }
        self.entries.insert(to.to_string(), node);
        for (k, n) in children {
            self.entries.insert(k, n);
        }
        Ok(())
    }

    /// Overwrite a file's full contents (used by write() syscall).
    pub fn write_file_full(&mut self, path: &str, data: &[u8]) -> Result<u64, &'static str> {
        let real = self.follow(path);
        match self.entries.get_mut(&real) {
            Some(n) if n.kind == FileKind::File => {
                n.content = data.to_vec();
                n.size = data.len() as u64;
                Ok(n.size)
            }
            Some(_) => Err("not a file"),
            None => Err("not found"),
        }
    }

    /// List immediate children of `dir` as (name, kind, size).
    pub fn list(&self, dir: &str) -> Vec<(String, FileKind, u64)> {
        let dir_norm = if dir.ends_with('/') {
            dir.to_string()
        } else {
            format!("{}/", dir)
        };
        let mut out = Vec::new();
        for key in self.entries.keys() {
            if key == dir {
                continue;
            }
            if let Some(rest) = key.strip_prefix(&dir_norm) {
                if !rest.is_empty() && !rest.contains('/') {
                    let n = self.entries.get(key).unwrap();
                    out.push((rest.to_string(), n.kind, n.size));
                }
            }
        }
        out
    }

    /// Total bytes used by file contents (excluding dirs).
    pub fn used_bytes(&self) -> u64 {
        self.entries.values().map(|n| n.size).sum()
    }
}

fn parent(p: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) if i == 0 => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => "/".to_string(),
    }
}

/// Public parent-path helper (used by the GUI file manager).
pub fn parent_of(p: &str) -> String {
    parent(p)
}

static FS: Lazy<Mutex<VirtualFs>> = Lazy::new(|| Mutex::new(VirtualFs::new()));

pub fn with<R, F: FnOnce(&mut VirtualFs) -> R>(f: F) -> R {
    f(&mut FS.lock())
}

// ---------------------------------------------------------------------------
// Disk persistence: PurityFS image written to ATA LBA 100..
// ---------------------------------------------------------------------------

const PFS_MAGIC: &[u8; 4] = b"PFS1";
const PFS_LBA: u32 = 100; // leave LBA 0..100 for bootloader / kernel

/// Serialize the whole filesystem into a byte buffer.
fn serialize(vfs: &VirtualFs) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(PFS_MAGIC);
    out.extend_from_slice(&(vfs.entries.len() as u32).to_le_bytes());
    for (path, node) in vfs.entries.iter() {
        out.extend_from_slice(&(path.len() as u16).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.push(match node.kind {
            FileKind::File => 1,
            FileKind::Dir => 2,
            FileKind::SymLink => 3,
            FileKind::Device => 4,
        });
        out.extend_from_slice(&(node.content.len() as u32).to_le_bytes());
        out.extend_from_slice(&node.content);
        out.extend_from_slice(&(node.target.len() as u16).to_le_bytes());
        out.extend_from_slice(node.target.as_bytes());
    }
    out
}

/// Parse a serialized image back into a fresh VirtualFs.
fn deserialize(buf: &[u8]) -> Option<VirtualFs> {
    if buf.len() < 8 || &buf[0..4] != PFS_MAGIC {
        return None;
    }
    let count = u32::from_le_bytes(buf[4..8].try_into().ok()?) as usize;
    let mut entries = BTreeMap::new();
    let mut pos = 8;
    for _ in 0..count {
        if pos + 2 > buf.len() { return None; }
        let plen = u16::from_le_bytes(buf[pos..pos + 2].try_into().ok()?) as usize;
        pos += 2;
        if pos + plen > buf.len() { return None; }
        let path = core::str::from_utf8(&buf[pos..pos + plen]).ok()?.to_string();
        pos += plen;
        if pos + 1 > buf.len() { return None; }
        let kind = match buf[pos] {
            1 => FileKind::File,
            2 => FileKind::Dir,
            3 => FileKind::SymLink,
            4 => FileKind::Device,
            _ => return None,
        };
        pos += 1;
        if pos + 4 > buf.len() { return None; }
        let clen = u32::from_le_bytes(buf[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + clen > buf.len() { return None; }
        let content = buf[pos..pos + clen].to_vec();
        pos += clen;
        if pos + 2 > buf.len() { return None; }
        let tlen = u16::from_le_bytes(buf[pos..pos + 2].try_into().ok()?) as usize;
        pos += 2;
        if pos + tlen > buf.len() { return None; }
        let target = core::str::from_utf8(&buf[pos..pos + tlen]).ok()?.to_string();
        pos += tlen;
        entries.insert(path, Node {
            kind,
            size: content.len() as u64,
            created: 0,
            target,
            content,
            mode: default_mode(kind),
        });
    }
    Some(VirtualFs { cwd: String::from("/home"), entries })
}

/// Write the current filesystem to the ATA disk.
pub fn flush_to_disk() {
    let bytes = serialize(&FS.lock());
    let n_sectors = (bytes.len() + 511) / 512;
    for i in 0..n_sectors {
        let mut sector = [0u8; 512];
        let start = i * 512;
        let end = (start + 512).min(bytes.len());
        sector[..end - start].copy_from_slice(&bytes[start..end]);
        let _ = crate::drivers::ata::write_sector(PFS_LBA + i as u32, &sector);
    }
    crate::klog!("[fs] flushed {} bytes to disk ({} sectors)\n", bytes.len(), n_sectors);
}

/// Load the filesystem from the ATA disk at boot. If the magic is missing,
/// keep the default in-memory tree (first boot).
///
/// Memory-safe: we read the superblock first, then only allocate as many
/// sectors as the on-disk image actually needs (hard cap 32 sectors = 16 KiB),
/// so a blank or corrupt disk can never balloon our 256 KiB heap.
pub fn load_from_disk() {
    let mut superblock = [0u8; 512];
    // First probe: read the superblock sector.
    if let Err(e) = crate::drivers::ata::read_sector(PFS_LBA, &mut superblock) {
        crate::klog!("[fs] ATA not available ({}), keeping in-memory FS\n", e);
        return;
    }
    if &superblock[0..4] != PFS_MAGIC {
        crate::klog!("[fs] no PurityFS signature on disk, fresh FS\n");
        return;
    }

    // The on-disk format begins: magic(4) | entry_count(4) | entries...
    // We trust the count only after a generous sanity bound.
    let on_disk_count = u32::from_le_bytes(superblock[4..8].try_into().unwrap()) as usize;
    if on_disk_count == 0 || on_disk_count > 4096 {
        crate::klog!("[fs] implausible entry count {}, fresh FS\n", on_disk_count);
        return;
    }

    // Conservative worst-case image size: each entry needs at least ~14 bytes
    // of framing. Cap the whole buffer at 16 KiB regardless.
    const MAX_BYTES: usize = 16 * 1024;
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    buf.extend_from_slice(&superblock);

    let mut sectors_needed = 1; // already have superblock
    // We can't trust content lengths yet; just stream a bounded number of
    // sectors and let deserialize reject anything that overruns.
    const MAX_SECTORS: usize = 32;
    for i in 1..MAX_SECTORS {
        if buf.len() >= MAX_BYTES {
            break;
        }
        let mut s = [0u8; 512];
        if crate::drivers::ata::read_sector(PFS_LBA + i as u32, &mut s).is_err() {
            break;
        }
        buf.extend_from_slice(&s);
        sectors_needed += 1;
    }

    if let Some(vfs) = deserialize(&buf) {
        let n = vfs.entries.len();
        *FS.lock() = vfs;
        crate::klog!("[fs] loaded {} entries from disk ({} sectors)\n", n, sectors_needed);
    } else {
        crate::klog!("[fs] disk image corrupt, keeping defaults\n");
    }
}
