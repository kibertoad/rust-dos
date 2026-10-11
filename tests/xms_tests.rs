//! The XMS driver (xms.rs) called directly, with the function in AH.

use iced_x86::Register;
use rust_dos::cpu::Cpu;
use rust_dos::xms;
use std::path::PathBuf;

/// Call the driver with function `ah`; returns AX.
fn xms(cpu: &mut Cpu, ah: u8) -> u16 {
    cpu.set_reg8(Register::AH, ah);
    xms::call(cpu);
    cpu.ax()
}

/// Lock the block `handle`: its address. Unlocks it again.
fn address(cpu: &mut Cpu, handle: u16) -> u32 {
    cpu.set_dx(handle);
    assert_eq!(xms(cpu, 0x0C), 1);
    let at = (cpu.dx() as u32) << 16 | cpu.bx() as u32;
    cpu.set_dx(handle);
    assert_eq!(xms(cpu, 0x0D), 1);
    at
}

#[test]
fn a_resize_past_4_gb_fails_and_keeps_the_block() {
    let mut cpu = Cpu::new(PathBuf::from("."));
    cpu.set_dx(64);
    assert_eq!(xms(&mut cpu, 0x09), 1);
    let handle = cpu.dx();
    // 1000000h KB is 16 GB, which wraps to nothing in 32 bits of bytes.
    cpu.set_dx(handle);
    cpu.set_ebx(0x0100_0000);
    assert_eq!(xms(&mut cpu, 0x8F), 0);
    assert_eq!(cpu.bx() as u8, 0xA0);
    cpu.set_dx(handle);
    assert_eq!(xms(&mut cpu, 0x8E), 1);
    assert_eq!(cpu.edx(), 64);
    // The block still holds its 64 KB: a new one goes after it.
    let first = address(&mut cpu, handle);
    cpu.set_dx(16);
    assert_eq!(xms(&mut cpu, 0x09), 1);
    let other = cpu.dx();
    let second = address(&mut cpu, other);
    assert!(second >= first + 64 * 1024, "{second:X} overlaps the block at {first:X}");
}
