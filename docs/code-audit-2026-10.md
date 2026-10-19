# Code audit: bugs, gaps and performance bottlenecks

Snapshot: upstream `main` at `ce1a1f2` (October 2026). Static review of the
whole tree (about 250k lines of Rust) by subsystem, every finding re-read on
that tree. Nothing here was fixed; this document is the list to work from.
Line numbers are those of `ce1a1f2` and will drift.

Severity is about impact on a user running a game, not code style:

- **High**: crash or hang of the emulator, data loss, a security hole, or a
  hot path visibly slowing games.
- **Medium**: wrong behaviour a real program can hit, or a measurable cost.
- **Low**: edge cases, hardening, and cheap wins.

Confidence is noted where a finding was not verified end to end.

## Top priorities

| # | Area | Finding | Kind |
|---|------|---------|------|
| 1 | Debug server | No origin, content-type or token check: any web page can drive `127.0.0.1:8086` | Security |
| 2 | Networking | Serial link that never comes up makes the network thread spin at 100% CPU | Perf |
| 3 | DOS | FindNext re-lists the whole directory every call: O(N²) per `DIR` | Perf |
| 4 | DOS | Every path component resolution lists and sorts the whole host directory | Perf |
| 5 | DPMI/XMS | Guest-controlled sizes overflow or allocate gigabytes (`0506h`, `0503h`, XMS `8Fh`) | Bug |
| 6 | CPU | System descriptors pass data-segment load checks; `mark_accessed` corrupts an LDT descriptor | Bug |
| 7 | CPU/FPU | 80-bit to f64 conversion truncates instead of rounding | Bug |
| 8 | Video | Planar (16-colour) modes re-decode every row every frame | Perf |
| 9 | Bus | PIT channel 2 has no counter state: speed calibration loops see random phase | Gap |
| 10 | Audio | AWE32 filter coefficients recomputed with four transcendentals per sample | Perf |

---

## 1. CPU interpreter and dynamic recompiler

Overall the ALU flag code, the x86-64 and AArch64 flag emulation, the TLB
fast paths, self-modifying-code detection and block invalidation all check
out against each other. Findings concentrate in protected-mode descriptor
checks and the 80-bit FPU helper.

### Bugs

- **Medium.** `src/cpu/seg.rs:85` `Descriptor::readable` is
  `is_data() || typ & 2 != 0` with no `is_segment()` test. Used by
  `check_data_segment` (`seg.rs:364`) and `load_task_segments`
  (`task.rs:273`), so a *system* descriptor with type bit 1 set (LDT = 2,
  busy TSS, interrupt/trap gates) loads into DS/ES/FS/GS instead of raising
  #GP. Worse, `mark_accessed` (`seg.rs:375`) then ORs bit 40 in and turns an
  LDT descriptor (type 2) into a busy 16-bit TSS (type 3), corrupting the
  guest's GDT. `VERR` in `instructions/system.rs:449` already does
  `is_segment() && readable()`, showing the intended check.
  Fix: require `is_segment()` in `readable()`.

- **Medium (precision).** `src/f80.rs:370-397` `F80::get_f64` converts the
  64-bit significand by `mantissa >> 11`, truncating instead of rounding to
  nearest even. `FpuRegs::set` stores this f64 as the working value for all
  non-add/sub FPU operations and the recompiler, so every 80-bit value that
  enters a register (`FLD tbyte`, exact `add_signed` results, `FBLD`, 64-bit
  integer loads) is rounded toward zero, biasing accumulations downward by
  up to 1 ulp per step. The x64 trampoline `fpu_addsub`
  (`src/dynrec/x64.rs:134-255`) mirrors the truncation to stay bit-identical.
  Fix: round-to-nearest-even on the dropped 11 bits with carry into the
  exponent, then mirror in the trampoline.

- **Low–Medium.** `src/f80.rs:379-383` `F80::get_f64` turns an 80-bit
  infinity into a NaN. The inf/NaN branch shifts the whole significand,
  integer bit included, so `7FFF 8000000000000000h` gives a non-zero
  `f64_mantissa` (bit 52) and the QNaN bit gets set. `F80::set_f64`
  stores infinity without the integer bit, so infinities the FPU makes
  itself round-trip, but `FLD tbyte` of a real x87 infinity (a constant
  in the program's data) becomes NaN in the working f64.
  Fix: decide NaN on the fraction without the integer bit
  (`mantissa << 1 != 0`) and mask bit 52 off as the normal branch does.

- **Low (panic in debug/test builds).** `src/f80.rs:300-312`
  `to_integer_u128` shifts a u128 left by `actual_exp - 63`. From 2^128
  up the high bits are shifted out silently, and from 2^191 up the shift
  amount reaches 128: a shift overflow panic under overflow checks,
  garbage in release. Reachable via `FBSTP`
  (`instructions/fpu/data.rs:195`). `to_bcd_packed` also stores
  `value mod 10^18` instead of the BCD indefinite for values ≥ 10^18.
  Fix: return the packed-BCD indefinite when `actual_exp >= 64` or the
  value exceeds 10^18 - 1.

- **Low.** `src/f80.rs:191-242` `add_signed` treats exponent `0x7FFF`
  operands as numbers: `inf - inf` yields +0 instead of the indefinite,
  and NaN payloads are shifted and summed. A carry out of exponent
  `0x7FFE` gives exponent `0x7FFF` with a non-zero fraction, a NaN where
  x87 gives infinity. The wrap into the sign that the comment at `:223`
  acknowledges comes from the other end: a cancellation whose shift `up`
  exceeds `exp` (results near the bottom of the range) makes
  `exp.wrapping_sub(up)` wrap to `FFxxh`, which sets the sign and an
  exponent near `7FFFh`, so a tiny difference becomes a huge number of
  the wrong sign.
  Fix: special-case inf/NaN before aligning significands, produce
  infinity on exponent overflow, and flush or denormalise on underflow.

- **Low.** `src/cpu/task.rs:175-180` loads CR3 from a 32-bit TSS only when
  CR0.PG is set; a 386 loads it unconditionally. An OS that task-switches
  with paging off and enables PG later runs with a stale CR3.
  Fix: always load CR3, flush the TLB only when PG is on.

- **Low (medium confidence).** `src/dynrec/x64.rs:2237-2273` `fpu_to_int`
  correctly gets `0x80000000` from `cvtsd2si` for out-of-range values but
  treats it as "doesn't fit" and falls back to `to_int`
  (`instructions/fpu/data.rs:83-86`), which uses Rust `as i32`/`as i16`
  saturation (positive overflow → `0x7FFF_FFFF`, NaN → 0) where x87 stores
  the integer indefinite (`0x8000_0000` / `0x8000`).
  Fix: return the indefinite in `to_int` for non-finite or out-of-range.

### Gaps

- **Low.** `src/cpu/mod.rs` `dr: [u32; 8]` is stored but DR0–DR3/DR7
  hardware breakpoints never fire; `fault.rs` only knows the single-step
  trap. Guest debuggers and some copy protections probing DR7 silently
  never break. Worth documenting as unsupported.

### Performance

- **Low.** `src/cpu/paging.rs:94-98` `Tlb::flush` fills 64 KB of entries on
  every CR3 load, CR0.PG/WP change and task switch, and `flush_page`
  (`paging.rs:103-120`) degrades to a 2048-entry scan per `INVLPG` once any
  4 MB page is cached. Guests that reload CR3 per context switch (Windows
  9x, multi-VM extenders) pay this constantly.
  Fix: a per-flush epoch in each entry, or tracking which 4 MB regions were
  inserted.

- **Low (cost not measured).** `src/dynrec/mod.rs:883` calls `vga_state()`
  on every entry into translated code, and `dynrec/helpers.rs:380, 590, 975`
  after every fallback instruction that may write memory, every translated
  `OUT` and every device write. It recomputes `Bus::plain_planes()`
  (`bus.rs:1381`) from VGA registers each time.
  Fix: a "planes dirty" flag set on VGA register writes, recompute lazily.

- **Low.** `src/dynrec/mod.rs:591` `Engine::find` runs `Bus::mark_code`'s
  chunk loop (`bus.rs:1199`) *before* the `none` table check at `:594`, so
  addresses where no block can start (service traps, HLT loops) pay
  `mark_code` on every miss. Move the `none` check first.

- **Low (translation time only).** `src/dynrec/block.rs:174-200`
  `BlockData::build` re-decodes the whole block up to several times while
  shrinking `max`, and the `side` closure (`dynrec/mod.rs:608-610`) fully
  translates each conditional jump, allocating a `Vec<Uop>`, just to test
  `is_some()`. This is the cost of every retranslation storm after
  `EXIT_STALE`/`EXIT_SMC`. A cheap `translatable(&Instruction)` predicate
  and a single decode with a cut-off would roughly halve it.

### Suspected, not confirmed

- `dynrec/helpers.rs:414, 434` `jit_fallback` clears `irq_shadow` right
  after STI / MOV SS when a next instruction exists in the block; if that
  instruction faults or bails, an interrupt may be taken one instruction
  early under `core=dynamic`.
- `cpu/fault.rs:303-307` and `cpu/farxfer.rs` call-gate paths write the
  new-stack frame before `mark_accessed` of the target code descriptor,
  which can #PF on the GDT page after the frame is written.
- `cpu/regs.rs:297` `load_seg_real` gives a real-mode CS a *data* access
  byte; no observable path found, but the recompiler's `flat_bit` keys on
  these bits.
- `src/dynrec/a64.rs` was compared with the x64 generator for `alu`,
  `shift`, `division_flags` and `divide` only (not compiled on this host).

---

## 2. Memory bus, system devices, DPMI, XMS, EMS

### Bugs

- **Medium.** `src/dpmi/int31.rs:416` (AX=0506h) collects
  `2 * ECX` bytes into a `Vec` before any bounds check. ECX=FFFFFFFFh from a
  protected-mode client makes the host try to allocate 8 GB with one
  `INT 31h`.
  Fix: write the `0009h` words straight through `cpu.write_u16` in a loop,
  stopping on the first fault, or validate ECX against the block's pages.

- **Medium.** `src/dpmi/int31.rs:480` (`resize_memory`, AX=0503h/0505h)
  and `src/dpmi/dos.rs:553` (`allocate_block`, AH=48h from a 32-bit client)
  compute `size.div_ceil(0x1000) * 0x1000` in u32; sizes above
  `FFFFF000h` pass `block_size`'s check (`dos.rs:545`) and overflow.
  Debug builds panic; release wraps `len` to 0 and records a 0-byte
  reservation while `tile()` still writes a descriptor with a ~4 GB limit,
  so the client gets a selector over all RAM and XMS treats the memory as
  free. (0501h/0504h are safe only because `take_dpmi` runs first with
  `checked_add`.)
  Fix: `checked_add(0xFFF).map(|n| n & !0xFFF)` as `xms::take_dpmi` does.

- **Medium.** `src/xms.rs:498` (`resize`, functions 0Fh/8Fh) and
  `src/xms.rs:162` (`gaps`): `size_kb * KB` is a u32 multiply of a guest
  register that overflows for `size_kb > 0x3F_FFFF`, so a resize to
  EBX=01000000h can succeed and be stored, after which `gaps()` computes
  wrapped ends and hands out overlapping blocks. Debug builds panic.
  Fix: compute in u64 and reject sizes above `memory_end`.

- **Medium.** `src/dpmi/mod.rs:1324` `push_lpms` does `sp -= size` on an
  ESP taken from the client when SS is the locked-stack selector
  (`keep_lpms`, `mod.rs:1166`). A client sitting at ESP < 12 on that stack
  when an interrupt arrives underflows: panic in debug, in release
  `base + sp` wraps to `base - 4` and the frame is written over the last
  descriptors of the client's LDT, which directly precedes the stack.
  Fix: `checked_sub`, abort the client on failure.

- **Low–Medium.** `src/mcb.rs:600` (`resize`) and `src/mcb.rs:679`
  (`coalesce`): `mcb_seg + 1 + m.size` is u16 arithmetic on a size read
  from guest memory. AH=49h/4Ah with ES at a fake `'M'` header of size
  `FFFFh` overflows (panic / bogus MCB in low memory). `walk_from`
  (`mcb.rs:237`) already uses `checked_add` for exactly this.
  Fix: `checked_add`, return `ERR_MCB_DESTROYED`.

- **Low–Medium.** `src/bus.rs:2648-2655` port 61h write folds bits 0 and
  1 into one flag (`speaker_on = value & 3 == 3`), and the read at
  `bus.rs:2977` rebuilds the byte from that flag. Writing 01h (gate on,
  speaker off) reads back 00h, so a program that sets the gate first and
  later turns the speaker on with `in al,61h; or al,2; out 61h,al` writes
  02h and stays silent. Any other value stored and read back comes out
  changed too.
  Fix: keep the last written byte, mask in live bits 4 and 5 on read.

### Gaps

- **Medium.** PIT channel 2 (`src/bus.rs:1721` `pit2_output`, `:2594`,
  `:2611-2641`, `:2918`) has no state beyond `pit_divisor`: output is
  synthesised from `now_ticks() % divisor`. Writing a count does not restart
  the counter, the gate (61h bit 0) never stops or starts it, the mode byte
  is ignored (one-shots read as a free-running square wave), and the
  channel-2 latch command does nothing (`_ => {}` at `:2641`) so LSB/MSB
  reads can tear. The classic "mode 0, gate, poll bit 5 / read back"
  CPU-speed and delay calibration gets random phase.
  Fix: reuse the `Pit0` model (`timer.rs`) for channel 2, gated by bit 0
  of the last 61h write.

- **Low.** `src/bus.rs:2611-2641` ignores PIT read-back (channel bits 11)
  and channel 1 entirely; port 41h falls to the open-bus `FFh`. Programs
  calibrating against the refresh counter get constants.

- **Low.** `src/pic.rs:130-150` ignores ICW4 special fully nested mode and
  OCW3 poll mode. A higher-priority slave IRQ (RTC, mouse) cannot be
  delivered while a lower one (IDE) is in service, unlike a real AT.

- **Low.** `src/cmos.rs:121-146` models only the periodic interrupt: alarm
  (AIE), update-ended (UIE) and the UIP bit never fire. Programs waiting on
  `INT 4Ah` alarms hang.

- **Low.** `src/mcb.rs:218-242` `walk_from` pushes a block before checking
  its end, so the last entry of a corrupt chain may extend past `A000h` and
  `alloc_in` can return a segment inside the video window.

### Performance

- **Medium.** `src/xms.rs:559-561` XMS `0Bh` move collects `length` bytes
  with per-byte `read_8`, then writes per-byte `write_8` (each bumping
  `page_gen`). DOS/4GW, Windows, disk caches and many games move 64 KB to
  several MB this way. Same pattern at `src/ems.rs:953-961` (function 57h)
  and `src/xms.rs:513-516` (resize relocation).
  Fix: when both ranges are `is_plain_ram`, use `copy_within` and one
  `bump_page_gens`; fall back to the byte loop only across the video window.

- **Low–Medium.** `src/bus.rs:2485-2790` `write_port` and `:2840-3010`
  `read_port` run a fixed chain of ownership guards (`verite_port`,
  `ide_claims` over both channels, `ne2000_claims`, `serial_claims`,
  `et4000()`, `sb_base()`, `gus_claims`, `awe_claims`, `Dma::owns`,
  `printer_claims`, `s3()`, `pci_present()`) before the big `match`, and
  finish with `self.vga.ports().contains(&port)` (a linear scan,
  `bus.rs:2749/3004`) for the most frequent ports of all (3C8h/3C9h/3D4h/
  3D5h). The `// TODO: Use a proper map lookup` at `bus.rs:2705`
  acknowledges it.
  Fix: a 64K-entry owner dispatch table rebuilt when devices are
  configured, or at least test the VGA ports first.

- **Low.** `src/ems.rs:182-199` `Ems::map` and `:136-170` `write_back`:
  each page map costs two 16 KB copies plus 512 `page_gen` bumps, and with
  aliases a byte-by-byte diff. Deliberate no-MMU design; programs remapping
  all four pages every frame pay ~128 KB of copies per frame. Noted for
  awareness only; the real fix is MMU-style translation.

- **Low.** `src/mcb.rs:748-779` `coalesce_chain` re-walks the whole chain
  (through `guest_read_*` and paging) after every single merge, and
  `coalesce` (`:675`) walks the chain to find the predecessor. Called on
  every AH=49h/4Ah and process end. A single forward pass merging runs of
  free blocks suffices.

### Checked and sound

`is_plain_ram`, `ram_read/write`, `read_8_mapped`, `write_8/16/32` bounds
and `unsafe` justification; VBE `window_offset` and `lfb_offset`;
`Bus::with_memory` clamping via `parse_memsize`; `pic.rs`, `kbc.rs`,
`timer.rs`, `bus/guest.rs`, `bus/state.rs`, `pci.rs`, `pnpbios.rs`,
`apm.rs`, `boot.rs`, `bios.rs`, keyboard ring buffer, mouse button
indices, `idle.rs` block copies; the `expect`/`unwrap` calls in
`dpmi/mod.rs` are behind `accepts()`/`start_host` preconditions.

---

## 3. Built-in DOS and storage

### Bugs

- **Medium.** `src/interrupts/int21.rs:1348-1363` AH=40h console branch
  sets AX but never clears CF, unlike the serial, printer and file branches
  beside it. A caller entering `INT 21h` with CF set (common after a
  compare) sees a spurious failure on `jc`, and `last_dos_error` is
  polluted because `handle()` records AX whenever CF is set.
  Fix: `cpu.set_cpu_flag(CpuFlags::CF, false)` after `set_ax`.

- **Medium (deliberate deviation, keep visible).**
  `src/interrupts/int21.rs:1147-1160` AH=3Ch and AX=6C00h action 2
  (`:2200`) map to a non-truncating `create_file`
  (`src/disk.rs:2352-2360`). The comment explains why: an eager truncate
  wiped saves when the emulator crashed mid-write. The cost is that the
  normal DOS idiom "create, write shorter content, close" leaves the old
  tail in place and produces a file the program then misparses. Now that
  AH=40h with CX=0 truncates, the rationale is weaker. The CX attributes are
  also ignored (marked TODO), so read-only/hidden files lose attributes on
  FAT images.
  Fix options: truncate on close if the program wrote anything, or truncate
  to the high-water mark; pass `attr` through to the volume.

- **Medium.** `src/disk.rs:2341-2347` `open_or_create` (host branch) maps
  every `OpenOptions::open` failure to `02h` "file not found", including
  permission denied on a read-only host file opened for write, sharing
  errors, and `EISDIR`. DOS returns `05h`; installers and save code branch
  on the difference.
  Fix: `PermissionDenied → 05h`, `NotFound → 02h`, others `05h`.

- **Low–Medium.** `src/interrupts/int21.rs:1389` sign-extends CX:DX
  (`as i32`) for every origin, so origin 0 with CX:DX ≥ `80000000h`
  reaches `seek_file` (`src/disk.rs:2644-2672`) as a negative offset:
  host files fail in `file.seek(SeekFrom::Start(offset as u64))` and the
  other backends in `seek_in` (`:2631`), both with `19h`. DOS treats
  CX:DX as unsigned for AL=0.
  Fix: in `int21.rs`, pass `((cx << 16) | dx) as i64` for origin 0 and
  keep the sign extension for origins 1 and 2.

- **Low.** `src/disk.rs:2606-2629` `write_file` and `read_file` (`:2564`)
  issue a single `write()`/`read()`, not a loop. On the libretro
  `hostfs::File::Foreign` backend short writes are legal; the partial count
  reaches the guest as AX < CX, which programs read as "disk full".

- **Low.** `src/interrupts/utils.rs:60-90` `read_dta_template` converts
  bytes ≥ `80h` with `c as char` (Latin-1) rather than CP437, so FCB/DTA
  names with high-bit characters fail to match host files that `dosstr`
  would decode correctly.

- **Low (panic in debug builds).** `src/cpu/mod.rs:2109-2145` `load_exe`
  uses unchecked u16 arithmetic on header fields
  (`0x10 + image_paras + min_alloc`, `load_segment + program_paras -
  image_paras`), and `image_data.len().div_ceil(16) as u16` truncates for
  images ≥ 1 MB. Malformed or large EXEs panic under overflow checks and
  load at a bogus segment in release.

### Gaps

- **Medium.** `src/interrupts/int13.rs:279-287` AH=02h/03h/04h on
  host-directory drives report success with AL = requested count without
  touching ES:BX; reads return whatever was in the buffer and writes are
  discarded. Intentional for presence checks, but sector-level tools get
  garbage with CF clear. At minimum zero-fill or synthesise a BPB from
  `layout().bpb()` on read, and fail writes with `03h`.

- **Medium.** `src/interrupts/int25.rs:82-90` INT 25h on host-mounted
  drives likewise returns success without filling the buffer (except one
  hard-coded word at `+1Ch` for MicroProse). Suggest synthesising sector 0
  from `FatLayout::bpb()` and zero-filling the rest.

- **Medium.** `src/interrupts/int21.rs:1562-1566` AH=44h "stub other
  subfunctions as success": AL=01h, 02h–05h (non-EMM), 06h (non-EMM),
  07h (non-EMM, non-PRN), 0Ah, 0Bh, 0Eh, 0Fh return AX=0, CF clear. For
  06h/07h that is AL=00h, "not ready", so a program that polls input or
  output status on a file or CON before reading or writing can wait
  forever. For 0Ah programs read DX bit 15 to decide "remote", and DX is
  left untouched. DOS returns `01h` (invalid function) for unknown
  subfunctions; 06h/07h (AL=FFh when ready, 00h at the end of an input
  file), 0Ah and 0Eh deserve explicit implementations.

- **Low.** `src/interrupts/int21.rs:2153` AH=68h/6Ah commit and `:1883`
  AH=0Dh disk reset do nothing: host files are never `sync_all()`ed and
  FAT volumes never flushed. A game that commits a save just before the
  emulator crashes loses data DOS would have written.

### Performance

- **High.** `src/disk.rs:3064-3078` `find_directory_entry` and
  `:3093-3215` `list_directory`, called from AH=4Eh/4Fh
  (`int21.rs:1695`) and FCB 11h/12h (`fcb.rs:198`). Every FindNext re-runs
  `list_directory` (full `read_dir`, sort, short-name computation, a
  `metadata` stat per entry on host drives or a cluster-chain walk on FAT
  images) and then `.nth(index)`. Enumerating N entries costs O(N²)
  syscalls; `DIR` on a few-thousand-file folder is visibly slow.
  Fix: cache the `Vec<DosDirEntry>` per search id (`search_handles`
  already keys on it), invalidate on directory writes.

- **High.** `src/disk.rs:1949-1973` `resolve_names_on` →
  `find_host_child` (`:2103`) → `host_entries` (`:2088`). Every path
  component reads and sorts the entire host directory and computes short
  names for all entries. Every open/exists/attribute/delete/rename is
  O(depth × dir_size) with heavy allocation, and callers chain them
  (`create_new_file` calls `exists` then `create_file`).
  Fix: per-directory `(long, short)` cache keyed by path + mtime. A
  `metadata(dir.join(target))` fast path is not safe for short names: on
  Windows, NTFS resolves its own 8.3 aliases, which need not match the
  ones `short_names` assigns (`PROGRA~1` can name another directory), so
  it would only do for targets the host name matches exactly.

- **Medium.** `src/fat.rs:1058-1097` `FatVolume::read`, `:1124-1162`
  `write`, `:503-517` `chain`, plus `disk.rs:2583, 2652` reloading the
  directory entry per read/seek. Each read walks the cluster chain from the
  first cluster to the current offset, each write materialises the whole
  chain into a `Vec<u32>`, and both go sector by sector. Sequential I/O on
  a disk image is O(n²) FAT lookups.
  Fix: keep a `(cluster, offset)` cursor in `OpenData::Fat`, read/write
  contiguous sector runs.

- **Medium (leak).** `src/interrupts/int21.rs:1844`
  `search_handles.insert(unique_id, dir)` (`bus.rs:257`) is never removed
  from. A program polling FindFirst in a loop grows the map without bound,
  and it is serialised into save states (`bus/state.rs`).
  Fix: drop the entry when FindNext returns `12h`, plus an LRU cap well
  above any real nesting depth. First make an unknown search id return
  `12h`: today FindNext falls back to `unwrap_or_default()`
  (`int21.rs:1777-1782`), the current directory, so a FindNext after the
  entry is gone would list files from another directory.

- **Low.** `src/interrupts/mscdex.rs:383-399` `read_long` writes CD
  sectors to guest memory one byte at a time via `guest_write_8`, and
  `count` (u16 from the request) is unbounded, allowing a 128 MB loop.
  Fix: `guest_write_bytes` per sector, cap `count`.

- **Low.** `src/disk.rs:2756-2789` `get_disk_free_space32` →
  `used_clusters` walks the entire host directory tree (`read_dir` plus
  `metadata` per file) on every AH=36h / AX=7303h for a floppy mounted from
  a host folder. Programs call this before every save.
  Fix: cache per drive, invalidate on write.

### Checked and sound

No `todo!`/`unimplemented!`; `unwrap`s in `fat.rs` are on fixed-size
slices; path traversal is blocked by `logical_components` dropping `..`
and the `starts_with(root)` check in `resolve_names_on` (also covers the
Windows `C:` component case); INT 13h CHS/LBA transfers are bounded by
`DiskImage::check` and `FatVolume::sector_range`; FCB record arithmetic
fits u32. Not audited in depth: `src/archive/**`, `src/cdrom/**` beyond
MSCDEX, `vhd.rs`, `d86f.rs`, `shared_disk.rs`, `crates/hostdirs`,
`crates/zip`, and the shell/batch commands (grepped for panics only).

---

## 4. Graphics

No guest-reachable panics found: `graphics_size`, `herc_graphics_shape`,
`S3::format`, `cga_geometry` and `text_cols` clamp register-derived sizes,
CRT timing code rejects zero line times before dividing, and VRAM indexing
is masked or `get()`-guarded throughout.

### Bugs and gaps

- **Low (DoS-ish).** `src/verite/twod.rs:221-248, 266-269` `visible()`
  tests only the clip rectangle and `>= 0`, not the surface, and
  `SET_CLIPPING` takes guest `i32`s unclamped. One `RECT_SOLID` with a
  32767×32767 clip issues ~10⁹ `plot` calls, each wrapping via `% n` over
  all of VRAM.
  Fix: clamp the clip to the surface in `SET_CLIPPING`/`SETUP`, bound
  `fill`'s ranges.

- **Low (medium confidence).** `src/video/vga.rs:804-814` `split_row`
  returns `line_compare / scanlines + 1`; DOSBox uses
  `(line_compare + 1) / scanlines`. These differ when `line_compare + 1` is
  not a multiple of scanlines-per-row (Line Compare = 200 in a
  double-scanned mode splits one row late). Verify with a mode-X
  split-screen test before changing.

- **Low.** `src/video/vga.rs:1113` ATC address write: bit 5 (palette
  address source / video enable) is noted in a comment but never stored or
  honoured, so programs that blank the screen while loading palettes or
  reprogramming the CRTC show partial frames instead of black.

- **Low (soundness).** `src/voodoo/raster.rs:560-585` `fetch_texel_in`
  uses `get_unchecked` on `t.lookup` and `t.ram` behind `debug_assert!`
  only. Sound today because every `Tmu::lookup` arm returns a 256- or
  65536-entry table and `mask + 1 == ram.bytes()`, but nothing enforces it
  at the type level; a later table change becomes silent UB in a hot loop.
  Fix: a one-time `assert!` where `TmuRaster` is built, or typed arrays.

### Performance

- **Medium.** `src/video/vga.rs:708, 775, 661, 593, 614` call
  `mark_dirty_full()` on every planar VRAM byte write, and `render_planar`
  (`src/video/mod.rs:567-620`) ignores the dirty band and re-decodes every
  row of the 16-colour modes (0Dh/0Eh/10h/12h and mode-X variants) each
  frame. The 256-colour path has a per-row compare and VBE has
  `Vbe::frame_rows`; planar has neither, so one sprite costs 640×480 pixels
  of re-conversion per frame.
  Fix: in `write_graphics`, map the written offset to a screen row the
  way `render_planar` reads VRAM (subtract `planar_base_offset`, divide
  by `bytes_per_row`, rows past `split_row` from address 0), mark nothing
  for writes to a hidden page, and mark everything when the start
  address, offset, split or panning changes; then honour `y_min..y_max`
  in `render_planar`. `plane_offset / bytes_per_row` alone is wrong as
  soon as a game scrolls or flips pages.

- **Medium.** `src/voodoo/gl.rs:774-786` `composite`, called every
  presented frame while the 3dfx picture is shown: a full-frame memcmp of
  `screen.rgb` vs `base.rgb`, then, whenever anything differs (cursor, text
  cursor, UI), a full-frame RGBA `Vec` allocation and a
  `tex_sub_image_2d` of the whole staging texture.
  Fix: persistent RGBA buffer, limit compare/upload to `changed_rows`
  (already computed at `display.rs:53`), or draw cursors as small quads.

- **Low–Medium.** `src/video/composite.rs:237, 269, 270` `decode_line`
  does three `vec!` allocations per scanline, 600 per frame in CGA
  composite mode. Keep scratch buffers in `Decoder` (`w` is always 640).

- **Low.** `src/display.rs:53, 395-412` with `src/main.rs:2065`: the
  pipeline copies the full rendered frame (`clone_from`) and rediscovers
  changed rows by comparing two full frames per presented frame, although
  `render_screen` already knows `dirty_y_min/max`. About 3 MB/frame of
  traffic at 1024×768.

- **Low.** `src/video/power.rs:211` `apply` clones the full framebuffer per
  frame while a monitor effect plays and does per-pixel f32 division for
  three channels. Bounded by effect length (≤ 1.6 s).

- **Low.** `src/capture/zmbv.rs:44, 51, 64-85` allocates a w×h×4 `Vec`
  per frame, clones it on keyframes, and grows `delta` by per-byte
  `extend`. On the recorder thread, so no emulation stall, but roughly
  doubles recording CPU. `mem::swap` with `previous`, `reserve` upfront.

- **Low.** Small per-frame allocations in hot renderers:
  `src/video/mod.rs:377` (`render_vbe` rebuilds a 256-entry colour `Vec`
  via 256 `get_rgb` calls even for one dirty row), `:588-589`, `:674`,
  `:746-769`, `src/video/s3/streams.rs:138`. Cache the DAC→RGB table on
  `VgaCard`, invalidated on 3C6h/3C9h writes.

- **Low.** `src/voodoo/mod.rs:944-964` software `render`: per-pixel bounds
  check, atomic `ram.get`, 3-byte palette copy. Only when the GL Voodoo
  renderer is off. Hoist the bound check per row, iterate `chunks_exact_mut`.

- **Low.** `src/powervr/render.rs:451-460, 337-348` `Cells::new` allocates
  12 `Vec`s per tile, ~3600 allocations per 640×480 render. Per-worker
  scratch reused across tiles.

---

## 5. Audio

The code is careful: no `todo!`, no guest-reachable index panics (every
register-indexed table was checked for size), and no callback-thread races
by construction since all mixing is on the emulation thread and SDL output
is an `AudioQueue`. Findings are gaps and medium-confidence issues.

### Bugs and gaps

- **Medium.** `crates/soundblaster/src/sb.rs:801` DSP command **E2h** (DMA
  identification) is unhandled and only logged. Creative's own drivers and
  installers and some detection routines program DMA and wait for the DSP
  to write a byte; here nothing arrives, so they hang or report "no card".
  The same `_ =>` arm swallows SB16 **45h/47h** (continue auto-init), so a
  transfer stopped by D9h/DAh cannot be resumed.
  Fix: implement E2h via `Dma::transfer_write` on the 8-bit channel; treat
  45h/47h as restarting auto-init.

- **Medium (medium confidence).** `crates/soundblaster/src/sb.rs:545-555`
  vs `:569`: SB Pro mixer writes (04h/22h/26h/28h) are mirrored into the
  SB16 registers 30h–37h, but SB16 writes are not mirrored back, so a
  driver mixing both APIs reads stale levels through the Pro view. DOSBox
  derives both views from one set of values.

- **Low–Medium (medium confidence).** `src/bus.rs:2017-2019`: when the SB
  output queue is empty at the start of a mixer frame, `sb_frame` snaps to
  `(dac, dac)` instead of holding the last sample. Any rounding or `frac`
  reset (`sb.rs:398` when the channel is briefly masked, a rate change
  mid-stream) that leaves the queue dry for a frame produces a step to DC,
  an audible click.
  Fix: hold while `sb.transfer.is_some()`.

- **Low.** `crates/soundblaster/src/sb.rs:402-403` recording transfers
  advance DMA with `transfer_skip` without writing silence, so the guest
  buffer keeps whatever it held; the moved count is also ignored, so
  `t.remaining` is decremented by the full `k` even when the channel masked
  itself part way, and the block-end IRQ comes early.

- **Low (save state).** `crates/mpu401/src/lib.rs:398` skips `held` and
  `mute`, and `after_load` replays only controllers/programs/sysex. The
  visualizer's held-note view is wrong after a load until every note is
  released.

- **Low.** `crates/soundblaster/src/sb.rs:749-758` D0h/D5h and D4h/D6h
  pause/resume whichever single transfer exists regardless of width.
  Harmless today; wrong once 8- and 16-bit transfers can coexist.

### Performance

- **Medium.** `crates/soundblaster/src/awe32/voice.rs:483-492`
  `Voice::filter` caches coefficients on exact equality of
  `(cutoff, filter_oct, filter_q)`, but `filter_oct` changes every sample
  from the modulation envelope and LFO1 (`run_envelopes`, `:646`). With any
  filter modulation (the norm for SoundFont drivers), every voice recomputes
  `exp2`, `sin`, `powf` and `log2` per sample: up to 32 × 44100 × 4
  transcendentals per second.
  Fix: quantise `filter_oct` (1/64 octave) before the cache compare, or
  recompute every N samples.

- **Low.** `crates/audio-core/src/dsp.rs:388` `Chorus::process` does two
  `sin()` calls per output frame per side for the LFO; a table or
  recurrence oscillator is cheap.

- **Low.** `src/audio.rs:177, 212` `pump_audio` allocates a fresh
  `Vec<i16>` for `samples` and `out` every video frame.

- **Low.** `crates/sc55/src/lib.rs:134` `Sc55::new` runs 0.5–2.5 s of
  emulated boot synchronously on the emulation thread when the config is
  applied, freezing the UI. Could run on a worker and be swapped in.

- **Low.** `crates/mpu401/src/midi_shadow.rs:97` `Vec::remove(0)` in a
  loop; bounded by `SYSEX_BUDGET`, a `VecDeque` makes it O(1).

### Checked and sound

8237 DMA count and word-channel addressing, auto-init reload; SB block
length, SB16 rate math, high-speed mode, time-constant math; sample
scaling and clamping; GUS position masks, bank addressing, `irq_chan %
active`, timer periods; `Feed::resample` boundaries and `pump_audio`
pad/drop arithmetic; save-state reader bounds (a hostile state cannot
force huge allocations); OPL timer semantics. Not audited: the SC-55
H8/sub-MCU/PCM cores instruction by instruction.

---

## 6. Host-side infrastructure: debug server, networking, recording, startup

### Bugs

- **High (security).** `src/debug/server.rs:434` `parse_body`, `:75`
  `router`, `:1117` `mount`, `:1149` `state_file`, `:1866` `ws_input`. The
  debug HTTP API accepts any `Content-Type`, validates neither `Origin` nor
  `Host`, has no CORS policy and no auth token. A POST with a `text/plain`
  body is a CORS "simple request", so **any web page open in the user's
  browser can drive the debug server on `127.0.0.1:8086`** (CSRF / DNS
  rebinding): mount arbitrary host directories as drives, write save-state
  files to arbitrary host paths via `/api/state/save`, type keys via
  `/api/input/*`, and open `/ws/input` (browsers do not enforce origin on
  WebSockets). `src/main.rs:112-113` also lets `--debug-server 0.0.0.0:8086`
  bind all interfaces without a warning.
  Fix: require `Content-Type: application/json` on mutating routes; reject
  requests whose `Origin` is present and not allowed and whose `Host` is
  not the bound address; and/or a random bearer token printed at startup.

- **Medium (panic).** `src/recorder.rs:75-76` `Encoder::new(...).unwrap()`
  and `set_repeat(...).unwrap()` run on the emulation thread at the first
  captured frame and do I/O on the just-created file (disk full, drive
  gone, permission revoked). Either aborts the emulator.
  Fix: propagate, stop the recording, show an OSD message as `write_frame`
  does.

- **Medium (DoS).** `src/net/tunnel/relay.rs:128, 282` and
  `src/net/tunnel/frag.rs:12, 55-59`: one `Reassembler` with
  `MAX_PENDING = 64` is shared by all members and rooms; when full, the
  oldest pending frame of anyone is evicted. One member sending 64 first
  fragments of distinct `seq`s (well under the 5000 pkt/s allowance)
  continuously evicts everyone else's half-assembled frames, so all
  > 1200-byte frames (normal 1514-byte Ethernet) are dropped relay-wide.
  Fix: per-member reassembler or per-member cap, evict only the offender's.

- **Medium (overflow).** `src/autoinput.rs:191` `wait`, reached from
  `(WAIT:n)` (`:177`) and `(DELAY:n)` (`:185`, then `:148, 153`) in a
  game profile or `.dosz` `run_input`, both parsed as any `u64` (`:48`):
  `now + ms * 1_000_000` overflows for `ms > ~1.8e13`, panicking in debug
  and wrapping to a past time in release. Input text comes from
  downloaded packages, so it is untrusted.
  Fix: clamp at parse time (≤ 1 h) or saturating arithmetic.

- **Medium (security, medium confidence).** `src/net/tunnel/client.rs:
  198-205`: while joined, an unauthenticated `REJECT Unknown/Closed`
  (token 0) is accepted if its source address equals the relay's. Anyone
  able to spoof the relay's UDP source address knocks every client off
  its room. Real on a hostile LAN.
  Fix: require the member's token on these rejects.

### Gaps

- **Medium.** `src/net/nat/host.rs:53-62` `Action::UdpOpen` ignores
  bind/registration failure silently, so the guest's UDP socket exists in
  the router but every `UdpSend` is dropped with no log. Emit a failure
  event so the flow closes and the user sees why UDP games/DNS fail.

- **Low.** `src/achievements/http.rs:41` `.expect("the RetroAchievements
  thread")` panics the emulator if the OS refuses a thread; the version
  check and phonebook spawn sites degrade gracefully.

- **Low.** `src/net/tunnel/discover.rs:56, 148` swallows non-timeout
  `recv_from` errors with `continue` (e.g. `ConnectionReset` from ICMP on
  Windows), spinning until the deadline. Bounded by `wait`.

### Performance

- **High.** `src/net/serial/link.rs:363-376` `next_due` with
  `src/net/hub.rs:263-269`. For a link that never came up (`up == false`,
  `theirs == 0`), after `DEAD_MS` (10 s) `next_due()` returns
  `min(last_heard + DEAD_MS, hello_due)`, a time in the past: `last_heard`
  only advances in `handle()`, and the restart branch at `link.rs:303`
  requires `up || theirs != 0`. The hub computes
  `due.saturating_sub(now) == 0` and `tokio::time::sleep(0)` fires every
  iteration, so the network thread spins at 100% CPU for as long as the
  roster peer does not answer HELLO (peer without a linked serial port,
  relay without `FEATURE_SERIAL`). The link exists whenever a roster peer
  exists (`station.rs:155, 217`).
  Fix: when `!up && theirs == 0` return just `hello_due`; or clamp `due`
  to `>= now` in the hub.

- **Medium.** `src/recorder.rs:100` `gif::Frame::from_rgb` runs NeuQuant
  quantisation at speed 1 (slowest) synchronously on the emulation thread
  for every captured frame (up to 15/s at 640×480 or more), stalling
  emulation while a GIF records.
  Fix: `from_rgb_speed(.., 10..30)` and/or a worker thread fed by a
  channel, as the printer and video recorder already do.

- **Low.** `src/net/serial/tcp.rs:72-88` → `src/net/serial/mod.rs:36`: the
  TCP pump pushes every 16 KiB read into an unbounded channel and then an
  unbounded `SerialQueue`; overflow is dropped only once the emulation
  thread drains. While paused (debugger, settings, rewind) a fast peer
  grows the queue without bound. `nat/host.rs` already has credit-based
  reading to reuse.

- **Low.** `src/main.rs:521`, `src/version_check.rs:111`: startup blocks
  up to 1 s for the version-check HTTP answer on the main thread.
  Intentional per the comments, but worth shortening or deferring to the
  notification path.

### Checked and sound

`net/tunnel/wire.rs` decode (lengths bounded, trailing bytes rejected,
amplification guarded); `frag.rs` index bounds; relay join cookie/HMAC
with constant-time compare, per-IP and per-room limits; NAT TCP credit
backpressure and connect timeouts; version check and phonebook body
limits on their own threads; `log.rs` size cap; `debug/journal.rs`
capacities; debug-server wait caps, `mem_get` length cap, WebSocket
streams gated by subscriber counters; `printer/escp.rs` tab indexing;
`import/dosbox.rs` and `config.rs` numeric parsing; `os_images.rs` name
validation; libretro FFI `CStr` uses.

---

## 7. Build hygiene

`cargo clippy --workspace --no-default-features` reports about 150 lint
warnings in the library, mostly `unnecessary_cast` (28), `precedence`
(24, all intentional shift/multiply mixes), `manual_is_multiple_of` (10)
and `identity_op` (9). None indicate bugs. Two `approx_constant` lints
in `tests/fpu_transcendental_tests.rs:77` are deny-by-default and fail
`cargo clippy --all-targets`; worth fixing if clippy is ever added to CI.

`Cargo.toml` has no `[profile]` section, so every overflow finding above
panics in debug and test builds and wraps silently in release. A
`overflow-checks = true` release profile for the guest-facing arithmetic,
or `checked_*` at the boundaries, would turn the silent cases into
diagnosable ones.

## Method

Six parallel reviewers, one per subsystem, each reading the code and
grepping for panicking constructs, unchecked arithmetic on guest-supplied
values, allocation in per-instruction/per-sample/per-frame paths, and
unbounded growth. Each finding was re-read on `ce1a1f2` after the
fast-forward from the fork, and the highest-impact ones were verified a
second time by the compiler of this document. Nothing was executed against
real programs; the performance findings are from reading, not profiling.
