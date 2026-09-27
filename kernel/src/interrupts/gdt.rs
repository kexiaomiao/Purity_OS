//! Global Descriptor Table + TSS (with a kernel stack for Ring 3 entry).

use spin::Lazy;
use x86_64::VirtAddr;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    // A 20 KiB double-fault stack.
    const STACK_SIZE: usize = 4096 * 5;
    #[repr(align(16))]
    struct Stack([u8; STACK_SIZE]);
    static mut STACK: Stack = Stack([0; STACK_SIZE]);
    let stack_start = VirtAddr::from_ptr(&raw const STACK as *const _);
    let stack_end = stack_start + STACK_SIZE as u64;
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_end;

    // Kernel stack used when a Ring 3 program traps into the kernel
    // (via int 0x80 or any interrupt). The CPU switches RSP to this
    // address automatically; without it we would triple-fault.
    const PRIV_STACK_SIZE: usize = 4096 * 8;
    #[repr(align(16))]
    struct PrivStack([u8; PRIV_STACK_SIZE]);
    static mut PRIV_STACK: PrivStack = PrivStack([0; PRIV_STACK_SIZE]);
    let priv_start = VirtAddr::from_ptr(&raw const PRIV_STACK as *const _);
    tss.privilege_stack_table[0] = priv_start + PRIV_STACK_SIZE as u64;

    tss
});

struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    user_data_selector: SegmentSelector,
    user_code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let data_selector = gdt.append(Descriptor::kernel_data_segment());
    let user_data_selector = gdt.append(Descriptor::user_data_segment());
    let user_code_selector = gdt.append(Descriptor::user_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(&TSS));
    (
        gdt,
        Selectors {
            code_selector,
            data_selector,
            user_data_selector,
            user_code_selector,
            tss_selector,
        },
    )
});

/// Load the GDT, reload CS/DS, and load the TSS.
pub fn init() {
    use x86_64::instructions::segmentation::{Segment, CS, DS};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        DS::set_reg(GDT.1.data_selector);
        load_tss(GDT.1.tss_selector);
    }
}

/// Kernel code segment selector (Ring 0).
pub fn kernel_code() -> SegmentSelector {
    GDT.1.code_selector
}

/// Kernel data segment selector (Ring 0).
pub fn kernel_data() -> SegmentSelector {
    GDT.1.data_selector
}

/// User data segment selector (Ring 3).
pub fn user_data() -> SegmentSelector {
    GDT.1.user_data_selector
}

/// User code segment selector (Ring 3).
pub fn user_code() -> SegmentSelector {
    GDT.1.user_code_selector
}
