/* PurityOS Ring 3 fork test: calls SYS_GETPID, SYS_FORK, prints which branch
   we are in (child fork() == 0, parent fork() == child pid), sleeps, exits.
   Freestanding; no libc. Entry is `_start`. */

typedef unsigned long long u64;

#define SYS_WRITE  1
#define SYS_EXIT   2
#define SYS_SLEEP  3
#define SYS_GETPID 9
#define SYS_FORK   13

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

static u64 sys_getpid(void)
{
    u64 r;
    __asm__ volatile("mov $9, %%rax; int $0x80" : "=a"(r) : : "memory");
    return r;
}

static u64 sys_fork(void)
{
    u64 r;
    __asm__ volatile("mov $13, %%rax; int $0x80" : "=a"(r) : : "memory");
    return r;
}

static void write(const char *s)
{
    u64 n = 0;
    while (s[n]) n++;
    sys_write(s, n);
}

static void write_num(u64 v)
{
    char buf[24];
    u64 i = sizeof(buf);
    do {
        buf[--i] = (char)('0' + (v % 10));
        v /= 10;
    } while (v);
    write(&buf[i]);
}

void _start(void)
{
    write("\n[fork] parent pid=");
    write_num(sys_getpid());
    write("\n");

    u64 r = sys_fork();
    if (r == 0) {
        /* Child branch. */
        write("[fork] CHILD pid=");
        write_num(sys_getpid());
        write(" fork() returned 0, sleeping 600ms...\n");
        sys_sleep(600);
        write("[fork] CHILD done, exiting.\n");
        sys_exit(0);
    } else {
        /* Parent branch. */
        write("[fork] PARENT fork() returned child pid=");
        write_num(r);
        write(", sleeping 900ms...\n");
        sys_sleep(900);
        write("[fork] PARENT done, exiting.\n");
        sys_exit(0);
    }
}
