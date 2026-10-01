# PurityOS v0.4.0

一个**从零开始、纯 Rust 编写的 x86_64 操作系统**：内核、驱动、调度器、命令行、
图形桌面、虚拟文件系统、Ring 3 用户态，全部自己实现，无裸机应用依赖。

```
  ██████╗ ██╗   ██╗██████╗ ██╗████████╗██╗   ██╗ ██████╗ ███████╗
  ██╔══██╗██║   ██║██╔══██╗██║╚══██╔══╝╚██╗ ██╔╝██╔═══██╗██╔════╝
  ██████╔╝██║   ██║██████╔╝██║   ██║    ╚████╔╝ ██║   ██║███████╗
  ██╔═══╝ ██║   ██║██╔══██╗██║   ██║     ╚██╔╝  ██║   ██║╚════██║
  ██║     ╚██████╔╝██║  ██║██║   ██║      ██║   ╚██████╔╝███████║
  ╚═╝      ╚═════╝ ╚═╝  ╚═╝╚═╝   ╚═╝      ╚═╝    ╚═════╝ ╚══════╝
```

---

## 特性总览

### 🖥️ 图形桌面（v0.4.0 新增）
- **1280×720 线性帧缓冲**（bootloader 提供），无 GPU 依赖
- 窗口管理器：圆角窗口、阴影、标题栏、关闭/最小化按钮、**拖拽移动**
- **macOS 风格顶栏**：应用名、版本、实时时钟、运行时间
- **Windows/UOS 风格任务栏**：Start 菜单、窗口按钮、系统托盘（鼠标坐标+时钟）
- **桌面图标** 8 个 + 渐变壁纸 + 鼠标光标（PS/2 驱动，IRQ12 中断）
- 主题切换（蓝/紫/绿/粉 4 套配色）
- 无帧缓冲时自动回退 VGA 文本模式，内核照常运行

### 📁 PurityFS v2（v0.4.0 重写）
- 4 种文件类型：`File / Dir / SymLink / Device`
- 符号链接（`ln`，8 跳解析上限防环）、设备节点（`dev`）
- 内置种子目录：`/home/docs/readme.md`、`/dev/tty`、`/dev/speaker`、符号链接 `/home/current`
- 命令：`ls pwd cd mkdir touch cat rm write ln dev fsinfo hexdump`
- GUI 文件管理器（Files 应用）直接浏览/打开 PurityFS

### 🧠 自动内存地址分配（v0.4.0 新增）
- **VirtAlloc**：first-fit 虚拟地址分配器，用户区 `0x0100_0000..0x7000_0000`
- 用户栈、用户程序段**全部自动取址**——**禁止硬编码地址**，杜绝内存冲突
- ELF 加载器把程序段映射到自动分配的页帧并标记 `USER_ACCESSIBLE`

### 🛡️ 用户态防崩（v0.4.0 新增）
- 用户程序在 Ring 3 崩溃（Page Fault / GP / 非法指令 / 除零）时：
  打印完整寄存器快照 → **杀掉用户程序 → 自动恢复回内核 shell**，OS 不死机
- `SYS_WRITE` 对用户指针做**边界校验**（拒绝内核地址越界读）

### 💻 命令行 shell（psh）
- Tab 补全、↑↓ 历史记录（环形缓冲）、`clear`、`help`
- RTC 真实时钟：`date` / `time`
- 内存：`free` / `mem` / `top` / `ps`
- 硬件：`lspci`（PCI 总线枚举）、`disk read <lba>`（ATA PIO 读扇区）
- 声音：`beep <hz> <ms>` / `play`（PIT 扬声器奏乐）
- 多任务：`spawn <name>`（内核线程）、`sleep <ms>`
- 用户态：`run`（加载并执行 Ring 3 程序）

### ⚙️ 内核基础设施
- GDT（用户段 DPL=3 + TSS rsp0）、IDT（异常/IRQ/`int 0x80` 软中断门 DPL=3）
- 8259 PIC 重映射、PIT 100Hz 时钟
- 分页（4 级页表 + BootInfoFrameAllocator + 256KiB 堆 + 用户页映射）
- **轮转调度器**：最多 8 个内核线程，PIT 中断触发切换，`hlt` 休眠
- **Ring 3**：ELF64 加载 → `iretq` 跳用户态 → `int 0x80` 系统调用
  （`SYS_WRITE=1 / SYS_EXIT=2 / SYS_SLEEP=3`）
- 异常寄存器打印器（用户崩溃时输出完整寄存器快照）

### 🎨 内置应用（8 个，GUI 点击即开）
| 应用 | 说明 |
|---|---|
| Terminal | 与内核 shell 联动的终端窗口 |
| PurityFS Files | 文件管理器（目录浏览、打开文件、↑ 返回上级） |
| Settings | 4 套主题切换 + 系统信息 |
| Clock | 实时大时钟 + 月历 |
| Calculator | 四则运算+括号，递归下降求值 |
| Paint | 240×180 画布 + 8 色调色板，拖拽绘画 |
| System Monitor | 运行时间/堆/用户虚拟内存/物理内存/任务列表 |
| Player | 3 支 PC 扬声器旋律 |

---

## 目录结构

```
PurityOS/
├── Cargo.toml            # workspace
├── xtask/                # 构建工具（打包磁盘镜像）
│   └── src/main.rs       # build / run：内核编译 + BIOS 镜像
└── kernel/
    ├── Cargo.toml
    ├── build.rs          # 编译 userspace/hello.c → user_hello.elf 嵌入内核
    ├── .cargo/config.toml
    ├── userspace/
    │   ├── hello.c       # Ring 3 用户程序（int 0x80 系统调用）
    │   └── user.ld
    └── src/
        ├── main.rs       # 入口：GDT/IDT/PIC/驱动/内存/GUI/调度
        ├── drivers/      # uart vga keyboard timer rtc speaker pci ata mouse
        ├── interrupts/   # gdt idt pic
        ├── mem/          # 分页 + 堆 + VirtAlloc（自动取址）
        ├── fs/           # PurityFS v2（File/Dir/SymLink/Device）
        ├── shell/        # psh：补全、历史、全部命令
        ├── task.rs       # 轮转调度 + 上下文切换（PurityOS_switch）
        ├── user.rs       # ELF 加载器、int 0x80、用户态防崩恢复
        └── gui/          # fb 帧缓冲 / font 点阵 / mouse / wm 窗口管理器
                          # apps 内置应用 / output 输出路由
```

---

## 构建与运行

```bash
# 依赖：Rust nightly（rust-toolchain 已锁定 1.100.0-nightly）
rustup component add rust-src llvm-tools-preview

# 构建并生成镜像（自动编译内核 → 打包 BIOS 启动镜像）
cargo run -p xtask -- build

# 或直接构建并启动 QEMU
cargo run -p xtask -- run
```

产物：`target/purityos-bios.img`（约 2.5 MiB，DOS/MBR 可引导镜像）。

也可手动用 QEMU 启动：

```bash
qemu-system-x86_64 -drive format=raw,file=target/purityos-bios.img -m 512M -serial stdio
```

> 启动后进入 GUI 桌面。点击桌面图标或 Start 菜单打开应用；点击 Terminal
> 窗口即可输入 shell 命令。若 QEMU 未提供帧缓冲，自动回退文本模式 shell。

---

## shell 命令速查

```
文件    ls pwd cd mkdir touch cat rm write hexdump ln dev fsinfo
信息    help echo clear version uptime date time mem free top ps
声音    beep <hz> <ms>   play
系统    lspci   disk read <lba>   spawn <name>   sleep <ms>
用户态  run               # 加载并执行内嵌的 Ring 3 程序
```

---

## 版本历史

- **v0.1.0** — 启动、VGA 文本 shell、键盘、PIT 时钟、RTC、扬声器、PCI、ATA
- **v0.2.0** — 内核线程 + 轮转调度 + RAMFS + 完整 shell
- **v0.3.0** — Ring 3：GDT 用户段、int 0x80、ELF 加载器、iretq、异常寄存器打印
- **v0.4.0** — 图形桌面 GUI、VirtAlloc 自动取址、PurityFS v2、8 个内置应用、
  用户态防崩、SYS_WRITE 边界校验

---

## 免责声明

PurityOS 是学习型操作系统，运行于 QEMU 虚拟机中。PCI/ATA/PS/2 驱动按标准
端口实现，但未在真实硬件上做过完整验证。仅供学习研究使用。
