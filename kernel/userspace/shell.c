/* PurityOS Ring 3 user shell: reads a line via SYS_READ, then either exec's
   /bin/hello via SYS_EXEC or exits. Freestanding, no libc. */

typedef unsigned long long u64;
typedef unsigned int u32;

static void sys_write(const char *s, u64 len)
{
    __asm__ volatile("mov %0, %%rdi; mov %1, %%rsi; mov $1, %%rax; int $0x80"
                     : : "r"((u64)s), "r"(len) : "rax", "rdi", "rsi", "memory");
}

static u64 sys_read(u64 fd, char *buf, u64 len)
{
    u64 ret;
    __asm__ volatile("mov %1, %%rdi; mov %2, %%rsi; mov %3, %%rdx; mov $4, %%rax; int $0x80"
                     : "=a"(ret) : "r"(fd), "r"((u64)buf), "r"(len)
                     : "rdi", "rsi", "rdx", "memory");
    return ret;
}

static u64 sys_exec(const char *path)
{
    u64 ret;
    __asm__ volatile("mov %1, %%rdi; mov $11, %%rax; int $0x80"
                     : "=a"(ret) : "r"((u64)path) : "rdi", "memory");
    return ret;
}

static void sys_exit(u64 code)
{
    __asm__ volatile("mov %0, %%rdi; mov $2, %%rax; int $0x80"
                     : : "r"(code) : "rax", "rdi", "memory");
    for (;;) { }
}

static void write(const char *s)
{
    u64 n = 0;
    while (s[n]) n++;
    sys_write(s, n);
}

static char line[256];

void _start(void)
{
    for (;;) {
        write("ush$ ");
        u64 got = sys_read(0, line, sizeof(line) - 1);
        if (got == 0) continue;
        line[got] = 0;

        /* Trim trailing newline / whitespace. */
        u64 i = got;
        while (i > 0 && (line[i-1] == '\n' || line[i-1] == '\r' || line[i-1] == ' ')) {
            line[--i] = 0;
        }
        if (i == 0) continue;

        if (line[0] == 'e' && line[1] == 'x' && line[2] == 'i' && line[3] == 't') {
            write("[ush] bye\n");
            sys_exit(0);
        }
        if (line[0] == 'e' && line[1] == 'c' && line[2] == 'h' && line[3] == 'o') {
            write(&line[5]);
            write("\n");
            continue;
        }
        if (line[0] == 'h' && line[1] == 'i') {
            char path[16] = "/bin/hello";
            sys_exec(path);
            write("[ush] exec failed\n");
            continue;
        }
        if (line[0] == 'f' && line[1] == 'o' && line[2] == 'r' && line[3] == 'k') {
            char path[16] = "/bin/forktest";
            sys_exec(path);
            write("[ush] exec failed\n");
            continue;
        }
        write("[ush] unknown command: ");
        write(line);
        write("\n");
    }
}
