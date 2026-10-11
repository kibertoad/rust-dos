//! INT 25h/26h — DOS absolute disk read and write.
//!
//! AL = drive (0 = A:), CX = sectors, DX = first sector and DS:BX = buffer;
//! or, for volumes over 32 MB, CX = FFFFh and DS:BX -> a packet of the first
//! sector (dword), the count (word) and the buffer (far pointer). Sectors
//! are numbered from the start of the drive's volume. The calls return
//! with the caller's flags still on the stack (see `return_from_hle`),
//! CF set and AX = error on failure: AH a BIOS status, AL a DOS critical
//! error code.
//!
//! Drives mounted from disk images read and write their sectors. The
//! others follow DOSBox: floppies and CD-ROMs fail, as if unreadable, and
//! hard disks pretend to succeed, reading as a boot sector made from the
//! drive's layout and zeros. From DOS 7.10 on, FAT32 drives take
//! INT 21h AX=7305h instead, as MS-DOS 7.1 has it.

use crate::cpu::{Cpu, CpuFlags};
use crate::disk::DriveKind;
use crate::diskimage::SECTOR_SIZE;
use crate::fat::FatType;

/// Error AX values.
const NOT_READY: u16 = 0x8002;
const BAD_REQUEST: u16 = 0x0207;
const WRITE_PROTECTED: u16 = 0x0300;

pub fn handle(cpu: &mut Cpu, write: bool) {
    let drive = cpu.get_al();
    let result = sector_access(cpu, drive, write, false);
    match result {
        Ok(()) => {
            cpu.set_ax(0);
            cpu.set_cpu_flag(CpuFlags::CF, false);
        }
        Err(error) => {
            cpu.set_ax(error);
            cpu.set_cpu_flag(CpuFlags::CF, true);
        }
    }
}

/// Read or write the sectors of `drive` (0 = A:) that CX, DX and DS:BX
/// ask for, as INT 25h/26h do, or (`fat32`) as INT 21h AX=7305h does with
/// its packet, which FAT32 drives take.
/// The boot sector of a drive mounted from a host folder, as DOS 4 and
/// later format one: its BPB, the extended boot record with the serial
/// number, label and file system that AX=440Dh/0866h report, and the
/// 55AAh signature.
fn host_boot_sector(cpu: &Cpu, drive: u8) -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];
    let Some(layout) = cpu.bus.disk.layout(drive) else { return sector };
    sector[..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    sector[3..11].copy_from_slice(b"MSDOS5.0");
    // The BPB up to the total sector count (25 bytes, 0Bh-23h).
    sector[0x0B..0x24].copy_from_slice(&layout.bpb()[..0x19]);
    sector[0x24] = 0x80;
    sector[0x26] = 0x29;
    let serial = cpu.bus.disk.volume_serial(drive).unwrap_or(crate::interrupts::int21::VOLUME_SERIAL + drive as u32);
    sector[0x27..0x2B].copy_from_slice(&serial.to_le_bytes());
    let label = cpu.bus.disk.volume_label(drive).filter(|l| !l.is_empty()).unwrap_or_else(|| "NO NAME".into());
    for (slot, b) in sector[0x2B..0x36].iter_mut().zip(label.bytes().chain(std::iter::repeat(b' '))) {
        *slot = b;
    }
    sector[0x36..0x3E].copy_from_slice(layout.fs_type());
    sector[0x1FE..].copy_from_slice(&[0x55, 0xAA]);
    sector
}

pub fn sector_access(cpu: &mut Cpu, drive: u8, write: bool, fat32: bool) -> Result<(), u16> {
    let packet = cpu.cx() == 0xFFFF;
    let (start, count, buffer) = if packet {
        let p = cpu.get_physical_addr(cpu.ds(), cpu.bx());
        let start = cpu.bus.guest_read_32(p as u32) as u64;
        let count = cpu.bus.guest_read_16((p + 4) as u32);
        let offset = cpu.bus.guest_read_16((p + 6) as u32);
        let segment = cpu.bus.guest_read_16((p + 8) as u32);
        (start, count, cpu.get_physical_addr(segment, offset))
    } else {
        (cpu.dx() as u64, cpu.cx(), cpu.get_physical_addr(cpu.ds(), cpu.bx()))
    };

    match (cpu.bus.disk.fat_volume(drive), cpu.bus.disk.drive_kind(drive)) {
        (Some(volume), _) => {
            let data_len = count as usize * SECTOR_SIZE;
            let refused = !fat32 && volume.fat_type() == FatType::Fat32 && cpu.bus.dos_version.at_least(7, 10);
            if refused || (!packet && volume.layout().total_sectors() > 0xFFFF) {
                Err(BAD_REQUEST)
            } else if write {
                if cpu.bus.disk.is_writable(drive) {
                    let mut data = vec![0u8; data_len];
                    cpu.bus.guest_read_bytes(buffer as u32, &mut data);
                    // A page of the buffer isn't there: the call runs
                    // again once it is, and nothing goes to the disk before.
                    if cpu.bus.guest_faulted() { Ok(()) } else { volume.write_sectors(start, &data) }
                } else {
                    Err(WRITE_PROTECTED)
                }
            } else {
                let mut data = vec![0u8; data_len];
                volume.read_sectors(start, &mut data).map(|()| cpu.bus.guest_write_bytes(buffer as u32, &data))
            }
            .inspect(|()| {
                let lba = volume.start() + start;
                cpu.bus.sector_activity(drive, lba, count as u32, write);
            })
        }
        (None, Some(DriveKind::HardDisk | DriveKind::Virtual)) => {
            // A host folder has no sectors: writes go nowhere, and reads
            // get a boot sector made from the drive's layout, with zeros
            // for the rest. MicroProse installers read C:'s for the hidden
            // sectors of its BPB.
            if !write {
                let mut data = vec![0u8; count as usize * SECTOR_SIZE];
                if start == 0
                    && let Some(sector) = data.get_mut(..SECTOR_SIZE)
                {
                    sector.copy_from_slice(&host_boot_sector(cpu, drive));
                }
                cpu.bus.guest_write_bytes(buffer as u32, &data);
            }
            Ok(())
        }
        _ => Err(NOT_READY),
    }
}
