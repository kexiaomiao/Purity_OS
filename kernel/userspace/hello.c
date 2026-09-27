/* PurityOS Ring 3 user program: prints a message via `int 0x80`, sleeps,
   prints again, then exits back to the kernel shell.
   Compiled freestanding; no libc, no CRT. Entry is `_start`. */

typedef unsigned long long u64;
typedef unsigned int u32;

/* Syscall numbers (must match kernel/src/interrupts/idt.rs). */
#define SYS_WRITE 1
#define SYS_EXIT  2
#define SYS_SLEEP 3

static void sys_write(const char *s, u64 len)
{
    __asm__ volatile("mov %0, %%rdi; mov %1, %%rsi; mov $1, %%rax; int $0x80"
                     : : "r"((u64)s), "r"(len) : "rax", "rdi", "rsi", "memory");
}

static void sys_sleep(u64 ms)
{
    __asm__ volatile("mov %0, %%rdi; mov $3, %%rax; int $0x80"
                     : : "r"(ms) : "rax", "rdi", "memory");
}

static void sys_exit(u64 code)
{
    __asm__ volatile("mov %0, %%rdi; mov $2, %%rax; int $0x80"
                     : : "r"(code) : "rax", "rdi", "memory");
    for (;;) { }
}

void _start(void)
{
    const char *a = "\n[user] hello from Ring 3!\n";
    const char *b = "[user] I am running in user mode, pid-less, but alive.\n";
    const char *c = "[user] slept for 1.5 s via SYS_SLEEP, still alive.\n";
    const char *d = "[user] exiting back to the kernel...\n";
    sys_write(a, sizeof(a) - 1);
    sys_write(b, sizeof(b) - 1);
    sys_sleep(1500);
    sys_write(c, sizeof(c) - 1);
    sys_write(d, sizeof(d) - 1);
    sys_exit(0);
}
