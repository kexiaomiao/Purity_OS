//! Physical/virtual memory management: page tables, frame allocator, heap.
//!
//! Frame allocation is globally persistent (`NEXT_FRAME`), so mappings made at
//! different times never reuse a physical frame.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bootloader_api::BootInfo;
use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use linked_list_allocator::LockedHeap;
use spin::Once;
use x86_64::VirtAddr;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame,
    Size4KiB,
};
use x86_64::PhysAddr;

/// The global heap allocator.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub const HEAP_START: usize = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 256 * 1024; // 256 KiB

/// Bootloader-provided physical memory offset, stored globally so we can
/// re-enter the page tables later (e.g. to map user pages).
static PHYS_OFFSET: AtomicU64 = AtomicU64::new(0);
/// Usable memory regions snapshot (part of BootInfo, 'static).
static MEM_REGIONS: Once<&'static [MemoryRegion]> = Once::new();
/// How many frames have been handed out in total (monotonic).
static NEXT_FRAME: AtomicUsize = AtomicUsize::new(0);

/// Frame allocator backed by the bootloader-provided memory map.
/// Tracks a global `next` counter so late mappings never double-assign frames.
pub struct BootInfoFrameAllocator {
    regions: &'static [MemoryRegion],
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Safety: the memory regions must describe all usable physical frames.
    pub unsafe fn new() -> Self {
        BootInfoFrameAllocator {
            regions: MEM_REGIONS.get().expect("memory regions not initialized"),
            next: NEXT_FRAME.load(Ordering::SeqCst),
        }
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        self.regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| (r.start..r.end).step_by(4096))
            .flatten()
            .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        NEXT_FRAME.store(self.next, Ordering::SeqCst);
        frame
    }
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let ptr: *mut PageTable = virt.as_mut_ptr();
    unsafe { &mut *ptr }
}

/// Build a fresh mapper for the current address space.
unsafe fn mapper() -> OffsetPageTable<'static> {
    let phys_offset = VirtAddr::new(PHYS_OFFSET.load(Ordering::SeqCst));
    let level_4 = unsafe { active_level_4_table(phys_offset) };
    unsafe { OffsetPageTable::new(level_4, phys_offset) }
}

/// Initialize memory management and the heap.
pub fn init(boot_info: &'static mut BootInfo) {
    let phys_offset = VirtAddr::new(
        boot_info
            .physical_memory_offset
            .into_option()
            .expect("bootloader did not map physical memory"),
    );
    PHYS_OFFSET.store(phys_offset.as_u64(), Ordering::SeqCst);

    // The memory_regions slice lives for 'static (it is part of BootInfo).
    let regions: &'static [MemoryRegion] = &boot_info.memory_regions;
    MEM_REGIONS.call_once(|| regions);

    let mut mapper = unsafe { mapper() };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::new() };

    map_heap(&mut mapper, &mut frame_allocator);

    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }
}

fn map_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE;
    let heap_start = VirtAddr::new(HEAP_START as u64);
    let heap_end = heap_start + HEAP_SIZE as u64 - 1u64;
    let start_page = Page::containing_address(heap_start);
    let end_page = Page::containing_address(heap_end);
    let page_range = Page::range_inclusive(start_page, end_page);

    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .expect("out of physical memory while mapping heap");
        unsafe {
            mapper
                .map_to(page, frame, flags, frame_allocator)
                .expect("failed to map heap page")
                .flush();
        }
    }
}

/// Map the virtual range `[vaddr, vaddr+len)` as **user-accessible** pages.
///
/// `writable` controls the U/W bit. The kernel's own pages (heap, code, data)
/// are never marked `USER_ACCESSIBLE`, so Ring 3 code cannot touch them.
pub fn map_user_pages(vaddr: usize, len: usize, writable: bool) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }

    let mut mapper = unsafe { mapper() };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::new() };

    let start = Page::containing_address(VirtAddr::new(vaddr as u64));
    let end = Page::containing_address(VirtAddr::new((vaddr + len - 1) as u64));
    // Roll back any partial mapping on failure so we don't leak physical frames.
    let mut mapped: alloc::vec::Vec<Page> = alloc::vec::Vec::new();
    let result = (|| -> Result<(), &'static str> {
        for page in Page::range_inclusive(start, end) {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or("out of physical memory")?;
            unsafe {
                mapper
                    .map_to(page, frame, flags, &mut frame_allocator)
                    .map_err(|_| "page mapping failed")?
                    .flush();
            }
            mapped.push(page);
        }
        Ok(())
    })();
    if result.is_err() {
        // Undo whatever got mapped so far.
        for page in mapped.drain(..) {
            if let Ok((_frame, flush)) = unsafe { mapper.unmap(page) } {
                flush.flush();
            }
        }
    }
    result
}

/// Zero a mapped virtual range (used for user segments / stacks).
pub fn zero_pages(vaddr: usize, len: usize) {
    unsafe { core::ptr::write_bytes(vaddr as *mut u8, 0, len) };
}

/// Copy bytes into a mapped virtual range.
pub fn copy_to(vaddr: usize, src: &[u8]) {
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), vaddr as *mut u8, src.len()) };
}

/// Heap usage stats: (allocated bytes, total heap bytes).
pub fn heap_stats() -> (usize, usize) {
    let alloc = ALLOCATOR.lock();
    (alloc.used(), alloc.size())
}

// ---------------------------------------------------------------------------
// Virtual address allocator (VirtAlloc)
// ---------------------------------------------------------------------------
//
// All dynamic address selection goes through this allocator. Nothing in the
// kernel hand-picks a virtual address anymore; regions are carved out of a
// free range automatically and tracked, so mappings can never collide.
//
// User virtual space layout:
//   [0x0040_0000 .. 0x0040_1000)  reserved for the single embedded user ELF
//                                 (its load address comes from the linker
//                                 script, not from the kernel)
//   [0x0100_0000 .. 0x7000_0000)  managed by VirtAlloc (stacks, heaps, maps)

/// Virtual-address allocator: first-fit over a fixed range, tracking used
/// spans so allocations never overlap.
pub struct VirtAlloc {
    base: usize,
    end: usize,
    cursor: usize,
    used: alloc::vec::Vec<(usize, usize)>, // sorted, non-overlapping
}

impl VirtAlloc {
    pub const fn new(base: usize, end: usize) -> Self {
        VirtAlloc { base, end, cursor: base, used: alloc::vec::Vec::new() }
    }

    /// Allocate `pages` contiguous 4 KiB pages. Returns the page-aligned
    /// virtual address, or `None` when the user range is exhausted.
    pub fn alloc(&mut self, pages: usize) -> Option<usize> {
        if pages == 0 {
            return None;
        }
        let need = pages * 4096;
        // First-fit scan starting at the cursor.
        let mut addr = self.cursor.max(self.base);
        loop {
            let mut a = addr;
            let mut blocked = false;
            for &(s, e) in &self.used {
                if a + need > s && a < e {
                    a = e; // jump past this used span
                    blocked = true;
                    break;
                }
            }
            if blocked {
                // a was pushed past an occupied span; rescan from there.
                if a >= self.end {
                    return None;
                }
                addr = a;
                continue;
            }
            // No overlap with any used span: check the range boundary.
            if a + need <= self.end {
                self.used.push((a, a + need));
                self.used.sort_unstable();
                self.cursor = if a + need + need > self.end {
                    self.base // wrap for later allocations
                } else {
                    a + need
                };
                return Some(a);
            }
            // Fits no span but does not fit before the end: out of space.
            return None;
        }
    }

    /// Return a previously allocated region.
    pub fn free(&mut self, addr: usize, pages: usize) {
        let e = addr + pages * 4096;
        self.used.retain(|&(s, x)| !(s == addr && x == e));
        self.cursor = self.base;
    }
}

static USER_ALLOC: spin::Mutex<VirtAlloc> =
    spin::Mutex::new(VirtAlloc::new(0x0100_0000, 0x7000_0000));

/// Allocate a user-accessible, zeroed region of `pages` pages at an
/// automatically chosen virtual address. Returns the base address.
pub fn alloc_user_region(pages: usize, writable: bool) -> Option<usize> {
    let addr = USER_ALLOC.lock().alloc(pages)?;
    match map_user_pages(addr, pages * 4096, writable) {
        Ok(()) => {
            zero_pages(addr, pages * 4096);
            Some(addr)
        }
        Err(_) => {
            USER_ALLOC.lock().free(addr, pages);
            None
        }
    }
}

/// How much user virtual address space is currently allocated (for `mem`).
pub fn user_alloc_stats() -> (usize, usize) {
    let a = USER_ALLOC.lock();
    let used: usize = a.used.iter().map(|&(s, e)| e - s).sum();
    (used, a.end - a.base)
}

/// Total usable physical memory in bytes (from the boot memory map).
pub fn phys_stats() -> u64 {
    match MEM_REGIONS.get() {
        Some(regions) => regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| r.end - r.start)
            .sum(),
        None => 0,
    }
}
