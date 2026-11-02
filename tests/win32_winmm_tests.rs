//! The built-in Windows' WINMM: a periodic timer counted over emulated
//! time, MIDI out reaching the MPU-401's synthesizer, MCI playing a CD
//! image's audio track, and RIFF files read and written with the mmio
//! functions. Each runs on the interpreter and on the dynamic
//! recompiler. The programs are PE files made with rust-dos-pe's
//! builder, their code assembled with iced.

mod cdimage;

use iced_x86::code_asm::*;
use rust_dos::cdrom::audio::PlayState;
use rust_dos::cpu::{CoreMode, Cpu, HostConsole};
use rust_dos::disk::MountOptions;
use rust_dos::exec::{NoHook, StopReason, run_batch};
use rust_dos_pe::{self as pe, Builder, Imp};
use std::fs;
use std::path::PathBuf;

const BASE: u32 = 0x40_0000;
const STD_OUTPUT_HANDLE: i32 = -11;
const CORES: [CoreMode; 2] = [CoreMode::Normal, CoreMode::Dynamic];

/// Where a test program's data and imports are, for its code.
struct Layout {
    data: u32,
    iat: Vec<u32>,
}

impl Layout {
    /// The address of byte `offset` of the data.
    fn at(&self, offset: u32) -> u64 {
        (self.data + offset) as u64
    }

    /// The import address table slot of the function `index`.
    fn import(&self, index: usize) -> AsmMemoryOperand {
        dword_ptr(self.iat[index] as u64)
    }
}

/// A program importing `imports`, with `data` and the code `f` makes.
fn program(
    imports: &[(&str, &[&str])],
    data: &[u8],
    f: impl FnOnce(&mut CodeAssembler, &Layout) -> Result<(), IcedError>,
) -> Vec<u8> {
    let mut b = Builder::exe(BASE);
    let lists: Vec<Vec<Imp>> = imports
        .iter()
        .map(|(_, names)| names.iter().map(|n| Imp::Name(n)).collect())
        .collect();
    let dlls: Vec<(&str, &[Imp])> = imports
        .iter()
        .zip(&lists)
        .map(|((dll, _), list)| (*dll, list.as_slice()))
        .collect();
    let iat = b.imports(&dlls).concat();
    let data_rva = b.section(
        ".data",
        data,
        pe::SCN_INITIALIZED_DATA | pe::SCN_READ | pe::SCN_WRITE,
    );
    let layout = Layout {
        data: BASE + data_rva,
        iat,
    };
    let code_rva = b.next_rva();
    let mut a = CodeAssembler::new(32).unwrap();
    f(&mut a, &layout).unwrap();
    let code = a.assemble((BASE + code_rva) as u64).unwrap();
    let flags = pe::SCN_CODE | pe::SCN_EXECUTE | pe::SCN_READ;
    assert_eq!(b.section(".text", &code, flags), code_rva);
    b.entry(code_rva);
    b.build()
}

/// The folder of a test's drive C:.
fn scratch_path(name: &str) -> PathBuf {
    PathBuf::from("target/test_win32_winmm").join(name)
}

/// A fresh folder for a test's drive C:, with `files` in it (a file's
/// name may have folders).
fn scratch(name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let base = scratch_path(name);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    for (file, bytes) in files {
        let path = base.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    base
}

/// A machine on `core` with the built-in Windows on, `program` loaded
/// from the shell as T.EXE, and what it prints kept. `prepare` runs
/// before the program is loaded.
fn machine(
    test: &str,
    core: CoreMode,
    files: &[(&str, Vec<u8>)],
    prepare: impl FnOnce(&mut Cpu),
) -> Cpu {
    let mut cpu = Cpu::new(scratch(test, files));
    cpu.core = core;
    cpu.bus.win32.enabled = true;
    cpu.host_console = Some(HostConsole::default());
    prepare(&mut cpu);
    cpu.load_shell();
    assert!(cpu.load_executable("T.EXE", None));
    cpu
}

/// Run one batch; whether the program ended and the shell is back.
fn batch(cpu: &mut Cpu) -> bool {
    cpu.bus.start_batch(cpu.bus.clock.icount + 100_000);
    run_batch(cpu, &mut NoHook, false) == StopReason::ShellReloaded
}

/// Run the machine until the program ends, for at most `batches`
/// batches, calling `watch` after each.
fn run_to_exit(cpu: &mut Cpu, batches: usize, mut watch: impl FnMut(&Cpu)) {
    for _ in 0..batches {
        let ended = batch(cpu);
        watch(cpu);
        if ended {
            return;
        }
    }
    panic!("the program didn't end: {:?}", printed(cpu));
}

/// What the program printed, to standard output and error.
fn printed(cpu: &Cpu) -> String {
    let console = cpu.host_console.as_ref().unwrap();
    let mut text = console.stdout.clone();
    text.extend(&console.stderr);
    String::from_utf8_lossy(&text).into_owned()
}

/// Put `bytes` at `offset` of `data`.
fn put(data: &mut [u8], offset: usize, bytes: &[u8]) {
    data[offset..offset + bytes.len()].copy_from_slice(bytes);
}

/// Call the import `slot` with `args`, pushed last first.
fn call(a: &mut CodeAssembler, slot: AsmMemoryOperand, args: &[i32]) -> Result<(), IcedError> {
    for &arg in args.iter().rev() {
        a.push(arg)?;
    }
    a.call(slot)
}

/// Set bit `bit` of EBX when EAX is `value`.
fn check_eax(a: &mut CodeAssembler, value: i32, bit: u32) -> Result<(), IcedError> {
    let mut skip = a.create_label();
    a.cmp(eax, value)?;
    a.jne(skip)?;
    a.or(ebx, 1 << bit)?;
    a.set_label(&mut skip)?;
    a.nop()
}

const TIMER_COUNT: u32 = 0x00;
const TIMER_START: u32 = 0x04;

/// A periodic timer of 10 ms counted by its callback over Sleep(105),
/// then killed. The exit code is the count, with bit 4 set if the
/// callback ran in the 50 ms after the kill, and bits 5 and 6 if
/// timeGetTime saw less than 105 ms or more than 120 ms pass over the
/// Sleep.
fn timer_program() -> Vec<u8> {
    const TIME_PERIODIC: i32 = 1;
    let imports: &[(&str, &[&str])] = &[
        ("KERNEL32.dll", &["Sleep", "ExitProcess"]),
        (
            "WINMM.dll",
            &["timeSetEvent", "timeKillEvent", "timeGetTime"],
        ),
    ];
    program(imports, &[0; 0x20], |a, l| {
        // The callback's address in EBP, which the CALL over it pushes.
        let mut start = a.create_label();
        a.call(start)?;
        // void CALLBACK TimeProc(UINT id, UINT msg, DWORD_PTR user,
        // DWORD_PTR dw1, DWORD_PTR dw2)
        a.inc(dword_ptr(l.at(TIMER_COUNT)))?;
        a.ret_1(20)?;
        a.set_label(&mut start)?;
        a.pop(ebp)?;
        a.call(l.import(4))?;
        a.mov(dword_ptr(l.at(TIMER_START)), eax)?;
        a.push(TIME_PERIODIC)?;
        a.push(0)?;
        a.push(ebp)?;
        a.push(1)?;
        a.push(10)?;
        a.call(l.import(2))?;
        a.mov(esi, eax)?;
        call(a, l.import(0), &[105])?;
        a.push(esi)?;
        a.call(l.import(3))?;
        a.call(l.import(4))?;
        a.sub(eax, dword_ptr(l.at(TIMER_START)))?;
        a.mov(edi, eax)?;
        a.mov(ebx, dword_ptr(l.at(TIMER_COUNT)))?;
        call(a, l.import(0), &[50])?;
        let mut quiet = a.create_label();
        a.cmp(ebx, dword_ptr(l.at(TIMER_COUNT)))?;
        a.je(quiet)?;
        a.or(ebx, 1 << 4)?;
        a.set_label(&mut quiet)?;
        let mut long_enough = a.create_label();
        a.cmp(edi, 105)?;
        a.jae(long_enough)?;
        a.or(ebx, 1 << 5)?;
        a.set_label(&mut long_enough)?;
        let mut short_enough = a.create_label();
        a.cmp(edi, 120)?;
        a.jbe(short_enough)?;
        a.or(ebx, 1 << 6)?;
        a.set_label(&mut short_enough)?;
        a.push(ebx)?;
        a.call(l.import(1))?;
        a.int3()
    })
}

#[test]
fn a_periodic_timer_fires_once_a_period_of_emulated_time() {
    for core in CORES {
        let mut cpu = machine("timer", core, &[("T.EXE", timer_program())], |_| {});
        run_to_exit(&mut cpu, 20_000, |_| {});
        // Sleep(105) holds 10 periods of 10 ms, and one more may come
        // due as the thread wakes.
        let code = cpu.errorlevel;
        assert!((10..=11).contains(&code), "{core:?}: exit code {code:#x}");
        assert!(printed(&cpu).is_empty(), "{:?}", printed(&cpu));
    }
}

/// MIDI out device 0 opened, a note played on channel 1 for 100 ms and
/// stopped, a second one on channel 2 left for midiOutReset, and the
/// device closed. A bit in the exit code for each call that worked.
fn midi_program() -> Vec<u8> {
    let imports: &[(&str, &[&str])] = &[
        ("KERNEL32.dll", &["Sleep", "ExitProcess"]),
        (
            "WINMM.dll",
            &[
                "midiOutOpen",
                "midiOutShortMsg",
                "midiOutReset",
                "midiOutClose",
            ],
        ),
    ];
    program(imports, &[0; 0x10], |a, l| {
        a.xor(ebx, ebx)?;
        call(a, l.import(2), &[l.at(0) as i32, 0, 0, 0, 0])?;
        check_eax(a, 0, 0)?;
        a.mov(esi, dword_ptr(l.at(0)))?;
        // Note on: key 60 at velocity 127, and key 64 at 100.
        a.push(0x7F_3C_90)?;
        a.push(esi)?;
        a.call(l.import(3))?;
        check_eax(a, 0, 1)?;
        a.push(0x64_40_91)?;
        a.push(esi)?;
        a.call(l.import(3))?;
        check_eax(a, 0, 2)?;
        call(a, l.import(0), &[100])?;
        // Note off for the first: a note on at velocity 0, in running
        // status after a status byte of its own.
        a.push(0x00_3C_90)?;
        a.push(esi)?;
        a.call(l.import(3))?;
        check_eax(a, 0, 3)?;
        call(a, l.import(0), &[100])?;
        a.push(esi)?;
        a.call(l.import(4))?;
        check_eax(a, 0, 4)?;
        a.push(esi)?;
        a.call(l.import(5))?;
        check_eax(a, 0, 5)?;
        a.push(ebx)?;
        a.call(l.import(1))?;
        a.int3()
    })
}

#[test]
fn midi_out_messages_reach_the_mpu_401s_synthesizer() {
    for core in CORES {
        let mut cpu = machine("midi", core, &[("T.EXE", midi_program())], |_| {});
        // What the synthesizer holds on channels 1 and 2, as it changes.
        let mut seen = vec![(0, 0)];
        run_to_exit(&mut cpu, 20_000, |cpu| {
            let held = cpu.bus.mpu.held_notes();
            let now = (held[0][60], held[1][64]);
            if seen.last() != Some(&now) {
                seen.push(now);
            }
        });
        assert_eq!(cpu.errorlevel, 0x3F, "{core:?}: {:?}", printed(&cpu));
        assert_eq!(seen, [(0, 0), (127, 100), (0, 100), (0, 0)], "{core:?}");
    }
}

const CD_COMMANDS: [&str; 8] = [
    "open cdaudio alias cd",
    "set cd time format tmsf",
    "play cd from 2",
    "status cd mode",
    "status cd position",
    "stop cd",
    "status cd mode",
    "close cd",
];

/// The MCI strings of CD_COMMANDS sent in turn, with 200 ms between the
/// play and the position. What each status returns is printed on a line
/// of its own; the exit code has a bit for each string that worked.
fn cd_program() -> Vec<u8> {
    const RET: u32 = 0x300;
    const CRLF: u32 = 0x340;
    const WRITTEN: u32 = 0x348;
    let mut data = vec![0u8; 0x350];
    for (i, command) in CD_COMMANDS.iter().enumerate() {
        put(&mut data, i * 0x40, command.as_bytes());
    }
    put(&mut data, CRLF as usize, b"\r\n");
    let imports: &[(&str, &[&str])] = &[
        (
            "KERNEL32.dll",
            &[
                "Sleep",
                "ExitProcess",
                "GetStdHandle",
                "WriteFile",
                "lstrlenA",
            ],
        ),
        ("WINMM.dll", &["mciSendStringA"]),
    ];
    program(imports, &data, |a, l| {
        a.xor(ebx, ebx)?;
        call(a, l.import(2), &[STD_OUTPUT_HANDLE])?;
        a.mov(edi, eax)?;
        for (i, command) in CD_COMMANDS.iter().enumerate() {
            if command.starts_with("status cd position") {
                call(a, l.import(0), &[200])?;
            }
            let args = [l.at(i as u32 * 0x40) as i32, l.at(RET) as i32, 64, 0];
            call(a, l.import(5), &args)?;
            check_eax(a, 0, i as u32)?;
            if command.starts_with("status") {
                call(a, l.import(4), &[l.at(RET) as i32])?;
                a.push(0)?;
                a.push(l.at(WRITTEN) as i32)?;
                a.push(eax)?;
                a.push(l.at(RET) as i32)?;
                a.push(edi)?;
                a.call(l.import(3))?;
                a.push(0)?;
                a.push(l.at(WRITTEN) as i32)?;
                a.push(2)?;
                a.push(l.at(CRLF) as i32)?;
                a.push(edi)?;
                a.call(l.import(3))?;
            }
        }
        a.push(ebx)?;
        a.call(l.import(1))?;
        a.int3()
    })
}

#[test]
fn mci_plays_an_audio_track_of_a_cd_image() {
    for core in CORES {
        let dir = cdimage::scratch("winmm_cd");
        let iso = cdimage::iso("TESTDISC", &[cdimage::file("A.TXT", b"a")]);
        // Five seconds of audio in track 2.
        let cue = cdimage::mixed_disc(&dir, &iso, 375);
        let mut cpu = machine("cd", core, &[("T.EXE", cd_program())], |cpu| {
            let options = MountOptions::default();
            cpu.bus.mount_drive(3, &cue, options, false).unwrap();
        });
        let mut played = false;
        run_to_exit(&mut cpu, 20_000, |cpu| {
            let player = &cpu.bus.cdaudio;
            played |= player.state() == PlayState::Playing && player.track() == Some(2);
        });
        let text = printed(&cpu);
        assert_eq!(cpu.errorlevel, 0xFF, "{core:?}: {text:?}");
        assert!(played, "{core:?}: the drive didn't play track 2");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{core:?}: {text:?}");
        assert_eq!(lines[0], "playing");
        // 200 ms is 15 of the 75 frames a second.
        assert_eq!(lines[1], "02:00:00:15");
        assert_eq!(lines[2], "stopped");
        assert_eq!(cpu.bus.cdaudio.state(), PlayState::Stopped, "{core:?}");
    }
}

// The program's data for the mmio test: two file names, and the
// MMCKINFOs (ckid, cksize, fccType, dwDataOffset, dwFlags) it reads and
// writes the chunks with.
const IN_NAME: u32 = 0x00;
const OUT_NAME: u32 = 0x10;
const READ_RIFF: u32 = 0x40;
const READ_CHUNK: u32 = 0x60;
const BUFFER: u32 = 0x80;
const TEXT: u32 = 0xA0;
const WRITE_RIFF: u32 = 0xC0;
const WRITE_CHUNK: u32 = 0xE0;

/// IN.WAV: a RIFF WAVE file of a 'fmt ' chunk and an 8-byte 'data' one.
fn riff_in() -> Vec<u8> {
    let mut file = b"RIFF".to_vec();
    file.extend(44u32.to_le_bytes());
    file.extend(b"WAVEfmt ");
    file.extend(16u32.to_le_bytes());
    file.extend([1, 0, 1, 0, 0x11, 0x2B, 0, 0, 0x11, 0x2B, 0, 0, 1, 0, 8, 0]);
    file.extend(b"data");
    file.extend(8u32.to_le_bytes());
    file.extend(b"12345678");
    file
}

/// IN.WAV's 'data' chunk found and read with mmioDescend and mmioRead,
/// and a search for its 'fmt ' chunk past it, which fails; then
/// OUT.WAV made with a RIFF WAVE chunk holding a 'data' chunk of three
/// bytes. Bits 0 to 6 of the exit code are the steps of the reading
/// that worked, and bit 7 is set if each of the writing's calls did.
fn mmio_program() -> Vec<u8> {
    const MMIO_WRITE: i32 = 1;
    const MMIO_CREATE: i32 = 0x1000;
    const MMIO_FINDCHUNK: i32 = 0x10;
    const MMIO_FINDRIFF: i32 = 0x20;
    const MMIO_CREATERIFF: i32 = 0x20;
    const MMIOERR_CHUNKNOTFOUND: i32 = 265;
    let fourcc = |s: &[u8; 4]| u32::from_le_bytes(*s) as i32;
    let mut data = vec![0u8; 0x100];
    put(&mut data, IN_NAME as usize, b"IN.WAV");
    put(&mut data, OUT_NAME as usize, b"OUT.WAV");
    put(&mut data, (READ_RIFF + 8) as usize, b"WAVE");
    put(&mut data, READ_CHUNK as usize, b"data");
    put(&mut data, TEXT as usize, b"abc");
    put(&mut data, (WRITE_RIFF + 8) as usize, b"WAVE");
    put(&mut data, WRITE_CHUNK as usize, b"data");
    let imports: &[(&str, &[&str])] = &[
        ("KERNEL32.dll", &["ExitProcess"]),
        (
            "WINMM.dll",
            &[
                "mmioOpenA",
                "mmioDescend",
                "mmioRead",
                "mmioAscend",
                "mmioCreateChunk",
                "mmioWrite",
                "mmioClose",
            ],
        ),
    ];
    program(imports, &data, |a, l| {
        let at = |offset: u32| l.at(offset) as i32;
        a.xor(ebx, ebx)?;
        call(a, l.import(1), &[at(IN_NAME), 0, 0])?;
        a.mov(esi, eax)?;
        a.test(eax, eax)?;
        a.setne(bl)?;
        a.push(MMIO_FINDRIFF)?;
        a.push(0)?;
        a.push(at(READ_RIFF))?;
        a.push(esi)?;
        a.call(l.import(2))?;
        check_eax(a, 0, 1)?;
        let find_chunk = |a: &mut CodeAssembler| -> Result<(), IcedError> {
            a.push(MMIO_FINDCHUNK)?;
            a.push(at(READ_RIFF))?;
            a.push(at(READ_CHUNK))?;
            a.push(esi)?;
            a.call(l.import(2))
        };
        find_chunk(a)?;
        check_eax(a, 0, 2)?;
        a.mov(eax, dword_ptr(l.at(READ_CHUNK + 4)))?;
        check_eax(a, 8, 3)?;
        a.push(8)?;
        a.push(at(BUFFER))?;
        a.push(esi)?;
        a.call(l.import(3))?;
        a.mov(eax, dword_ptr(l.at(BUFFER + 4)))?;
        check_eax(a, fourcc(b"5678"), 4)?;
        a.push(0)?;
        a.push(at(READ_CHUNK))?;
        a.push(esi)?;
        a.call(l.import(4))?;
        check_eax(a, 0, 5)?;
        a.mov(dword_ptr(l.at(READ_CHUNK)), fourcc(b"fmt "))?;
        find_chunk(a)?;
        check_eax(a, MMIOERR_CHUNKNOTFOUND, 6)?;
        a.push(0)?;
        a.push(esi)?;
        a.call(l.import(7))?;

        // Each call of the writing jumps to `failed` if it fails.
        let mut failed = a.create_label();
        call(a, l.import(1), &[at(OUT_NAME), 0, MMIO_CREATE | MMIO_WRITE])?;
        a.mov(esi, eax)?;
        a.test(eax, eax)?;
        a.jz(failed)?;
        let steps: [(usize, [i32; 3], i32); 6] = [
            (5, [at(WRITE_RIFF), MMIO_CREATERIFF, 0], 0),
            (5, [at(WRITE_CHUNK), 0, 0], 0),
            (6, [at(TEXT), 3, 0], 3),
            (4, [at(WRITE_CHUNK), 0, 0], 0),
            (4, [at(WRITE_RIFF), 0, 0], 0),
            (7, [0, 0, 0], 0),
        ];
        for (import, args, ok) in steps {
            // mmioClose takes two arguments, the others three.
            let count = if import == 7 { 1 } else { 2 };
            for &arg in args[..count].iter().rev() {
                a.push(arg)?;
            }
            a.push(esi)?;
            a.call(l.import(import))?;
            a.cmp(eax, ok)?;
            a.jne(failed)?;
        }
        a.or(ebx, 1 << 7)?;
        a.set_label(&mut failed)?;
        a.push(ebx)?;
        a.call(l.import(0))?;
        a.int3()
    })
}

#[test]
fn mmio_reads_and_writes_riff_chunks() {
    for core in CORES {
        let files = [("T.EXE", mmio_program()), ("IN.WAV", riff_in())];
        let mut cpu = machine("mmio", core, &files, |_| {});
        run_to_exit(&mut cpu, 20_000, |_| {});
        assert_eq!(cpu.errorlevel, 0xFF, "{core:?}: {:?}", printed(&cpu));
        let out = fs::read(scratch_path("mmio").join("OUT.WAV")).unwrap();
        // The 'data' chunk's odd size is padded to a word.
        let mut expected = b"RIFF".to_vec();
        expected.extend(16u32.to_le_bytes());
        expected.extend(b"WAVEdata");
        expected.extend(3u32.to_le_bytes());
        expected.extend(b"abc\0");
        assert_eq!(out, expected, "{core:?}");
    }
}

// The sequencer test's data: its two MCI strings, the alias, an
// MCI_SEQ_SET_PARMS (dwCallback, dwTimeFormat, dwAudio, dwTempo, dwPort,
// dwSlave, dwMaster, dwOffset) and an MCI_STATUS_PARMS (dwCallback,
// dwReturn, dwItem, dwTrack).
const SEQ_OPEN: u32 = 0x00;
const SEQ_CLOSE: u32 = 0x40;
const SEQ_ALIAS: u32 = 0x60;
const SEQ_PARMS: u32 = 0x80;
const SEQ_PORT: u32 = SEQ_PARMS + 16;
const SEQ_STATUS: u32 = 0xA0;
const SEQ_RETURN: u32 = SEQ_STATUS + 4;
const SEQ_ITEM: u32 = SEQ_STATUS + 8;

/// A Standard MIDI File of one empty track.
fn empty_midi() -> Vec<u8> {
    let mut file = b"MThd".to_vec();
    file.extend(6u32.to_be_bytes());
    file.extend([0, 0, 0, 1, 0, 96]);
    file.extend(b"MTrk");
    file.extend(4u32.to_be_bytes());
    file.extend([0x00, 0xFF, 0x2F, 0x00]);
    file
}

/// M.MID opened on the sequencer, its port set to Win32's MIDI_MAPPER,
/// (UINT)-1, then to port 0, then to port 2, which doesn't exist, and
/// closed, with MCI_STATUS of the port after each set. Bits of the exit
/// code: 0, the open worked; 1 and 2, the mapper was taken and reported
/// as 65535, the 16-bit driver's word; 3 and 4, port 0 was taken and
/// reported; 5 and 6, port 2 was out of range and port 0 stayed; 7, the
/// close worked.
fn sequencer_program() -> Vec<u8> {
    let mut data = vec![0u8; 0xC0];
    put(
        &mut data,
        SEQ_OPEN as usize,
        b"open M.MID type sequencer alias m",
    );
    put(&mut data, SEQ_CLOSE as usize, b"close m");
    put(&mut data, SEQ_ALIAS as usize, b"m");
    const MCI_SEQ_STATUS_PORT: u32 = 0x4003;
    put(
        &mut data,
        SEQ_ITEM as usize,
        &MCI_SEQ_STATUS_PORT.to_le_bytes(),
    );
    let imports: &[(&str, &[&str])] = &[
        ("KERNEL32.dll", &["ExitProcess"]),
        (
            "WINMM.dll",
            &["mciSendStringA", "mciGetDeviceIDA", "mciSendCommandA"],
        ),
    ];
    const MCI_SET: i32 = 0x80D;
    const MCI_STATUS: i32 = 0x814;
    const MCI_SEQ_SET_PORT: i32 = 0x0002_0000;
    const MCI_STATUS_ITEM: i32 = 0x100;
    const MCIERR_OUTOFRANGE: i32 = 282;
    let sets = [
        (-1, 0, 65535, 1),
        (0, 0, 0, 3),
        (2, MCIERR_OUTOFRANGE, 0, 5),
    ];
    program(imports, &data, |a, l| {
        a.xor(ebx, ebx)?;
        call(a, l.import(1), &[l.at(SEQ_OPEN) as i32, 0, 0, 0])?;
        check_eax(a, 0, 0)?;
        call(a, l.import(2), &[l.at(SEQ_ALIAS) as i32])?;
        a.mov(esi, eax)?;
        for (port, result, reported, bit) in sets {
            a.mov(dword_ptr(l.at(SEQ_PORT)), port)?;
            a.push(l.at(SEQ_PARMS) as i32)?;
            a.push(MCI_SEQ_SET_PORT)?;
            a.push(MCI_SET)?;
            a.push(esi)?;
            a.call(l.import(3))?;
            check_eax(a, result, bit)?;
            a.mov(dword_ptr(l.at(SEQ_RETURN)), -2)?;
            a.push(l.at(SEQ_STATUS) as i32)?;
            a.push(MCI_STATUS_ITEM)?;
            a.push(MCI_STATUS)?;
            a.push(esi)?;
            a.call(l.import(3))?;
            a.test(eax, eax)?;
            let mut failed = a.create_label();
            a.jne(failed)?;
            a.mov(eax, dword_ptr(l.at(SEQ_RETURN)))?;
            check_eax(a, reported, bit + 1)?;
            a.set_label(&mut failed)?;
            a.nop()?;
        }
        call(a, l.import(1), &[l.at(SEQ_CLOSE) as i32, 0, 0, 0])?;
        check_eax(a, 0, 7)?;
        a.push(ebx)?;
        a.call(l.import(0))?;
        a.int3()
    })
}

#[test]
fn the_sequencer_takes_win32s_midi_mapper_as_its_port() {
    for core in CORES {
        let files = [("T.EXE", sequencer_program()), ("M.MID", empty_midi())];
        let mut cpu = machine("sequencer", core, &files, |_| {});
        run_to_exit(&mut cpu, 20_000, |_| {});
        assert_eq!(cpu.errorlevel, 0xFF, "{core:?}: {:?}", printed(&cpu));
    }
}

const AUX_CAPS: u32 = 0x00;
const AUX_VOLUME: u32 = 0x40;

/// The auxiliary devices as Test Drive 4 uses them: auxSetVolume on
/// device 1 with no auxGetNumDevs first, then the CD's device and the
/// microphone's, which is mono. A bit in the exit code for each call
/// that returned what Windows 95's Sound Blaster 16 driver does.
fn aux_program() -> Vec<u8> {
    const MMSYSERR_BADDEVICEID: i32 = 2;
    const AUXCAPS_CDAUDIO: i32 = 1;
    let imports: &[(&str, &[&str])] = &[
        ("KERNEL32.dll", &["Sleep", "ExitProcess"]),
        (
            "WINMM.dll",
            &[
                "auxGetNumDevs",
                "auxGetDevCapsA",
                "auxGetVolume",
                "auxSetVolume",
            ],
        ),
    ];
    program(imports, &[0; 0x50], |a, l| {
        a.xor(ebx, ebx)?;
        call(a, l.import(5), &[1, 0x1FFF_1FFF])?;
        check_eax(a, 0, 0)?;
        a.call(l.import(2))?;
        check_eax(a, 7, 1)?;
        call(a, l.import(5), &[2, 0x4000_2000])?;
        check_eax(a, 0, 2)?;
        call(a, l.import(5), &[7, 0])?;
        check_eax(a, MMSYSERR_BADDEVICEID, 3)?;
        // Device 2 is the CD's: AUXCAPS' wTechnology, at 40, says so.
        call(a, l.import(3), &[2, l.at(AUX_CAPS) as i32, 48])?;
        check_eax(a, 0, 4)?;
        a.movzx(eax, word_ptr(l.at(AUX_CAPS + 40)))?;
        check_eax(a, AUXCAPS_CDAUDIO, 5)?;
        // There is no aux mapper.
        call(a, l.import(3), &[-1, l.at(AUX_CAPS) as i32, 48])?;
        check_eax(a, MMSYSERR_BADDEVICEID, 6)?;
        // The microphone keeps the volume it is given.
        call(a, l.import(5), &[4, 0x4000_2000])?;
        call(a, l.import(4), &[4, l.at(AUX_VOLUME) as i32])?;
        a.mov(eax, dword_ptr(l.at(AUX_VOLUME)))?;
        check_eax(a, 0x4000_2000, 7)?;
        // A while for the test to see the volumes before the exit.
        call(a, l.import(0), &[100])?;
        a.push(ebx)?;
        a.call(l.import(1))?;
        a.int3()
    })
}

/// Device 1 is the MIDI synthesizer's volume, as on a Sound Blaster 16,
/// and device 2 the CD's: what they are set to scales what each plays.
#[test]
fn aux_devices_are_the_sound_blaster_16s_lines() {
    for core in CORES {
        let mut cpu = machine("aux", core, &[("T.EXE", aux_program())], |_| {});
        let mut seen = Vec::new();
        run_to_exit(&mut cpu, 20_000, |cpu| {
            let now = (cpu.bus.win32.midi_gain(), cpu.bus.win32.cd_gain());
            if seen.last() != Some(&now) {
                seen.push(now);
            }
        });
        assert_eq!(cpu.errorlevel, 0xFF, "{core:?}: {:?}", printed(&cpu));
        let level = |v: u32| v as f32 / 65535.0;
        let set = (
            (level(0x1FFF), level(0x1FFF)),
            (level(0x2000), level(0x4000)),
        );
        assert!(seen.contains(&set), "{core:?}: {seen:?}");
    }
}

// An installable MCI driver, TESTV32.DLL, at its own base. Its data is
// a log of the messages its DriverProc got (a count, then up to 16), a
// word set when an MCI message came with another driver id than the one
// its DRV_OPEN returned, the first character of the element its
// MCI_OPEN_DRIVER got, the driver data MCI_STATUS read back and the
// device id DRV_OPEN was given.
const DRIVER_BASE: u32 = 0x1000_0000;
const LOG_COUNT: u32 = 0x00;
const LOG: u32 = 0x04;
const BAD_ID: u32 = 0x48;
const ELEMENT: u32 = 0x4C;
const DRIVER_DATA: u32 = 0x50;
const DEVICE_ID: u32 = 0x54;
const DRIVER_ID: i32 = 0x55;

const DRV_OPEN: i32 = 3;
const MCI_OPEN_DRIVER: i32 = 0x801;
const MCI_CLOSE_DRIVER: u32 = 0x802;
const MCI_OPEN: i32 = 0x803;
const MCI_CLOSE: i32 = 0x804;
const MCI_INFO: i32 = 0x80A;
const MCI_STATUS: i32 = 0x814;
const MCI_OPEN_ELEMENT: i32 = 0x200;
const MCI_INFO_PRODUCT: i32 = 0x100;
const MCI_STATUS_ITEM: i32 = 0x100;
const MCI_STATUS_MODE: u32 = 4;
const MCI_MODE_PLAY: i32 = 526;
const MCI_DEVTYPE_DIGITAL_VIDEO: i32 = 521;
const MCIERR_INVALID_DEVICE_ID: i32 = 257;

/// The driver: a DLL whose DriverProc answers DRV_OPEN with its device
/// type and a command table of its own (RCDATA 5), takes MCI_OPEN_DRIVER
/// and keeps driver data, gives "Test" in UTF-16 for MCI_INFO and a mode
/// for MCI_STATUS with the resource flag (MCI_RESOURCE_RETURNED), and
/// logs every message. It exports DriverProc and Log, which gives the
/// log's address.
fn mci_driver() -> Vec<u8> {
    let mut b = Builder::dll(DRIVER_BASE, "TESTV32.DLL");
    let names = ["mciLoadCommandResource", "mciSetDriverData", "mciGetDriverData"];
    let list: Vec<Imp> = names.iter().map(|n| Imp::Name(n)).collect();
    let iat = b.imports(&[("WINMM.dll", list.as_slice())]).concat();
    let data_rva = b.section(
        ".data",
        &[0; 0x60],
        pe::SCN_INITIALIZED_DATA | pe::SCN_READ | pe::SCN_WRITE,
    );
    let data = DRIVER_BASE + data_rva;
    let at = |offset: u32| dword_ptr((data + offset) as u64);
    let import = |i: usize| dword_ptr(iat[i] as u64);
    let code = |f: &dyn Fn(&mut CodeAssembler) -> Result<(), IcedError>, rva: u32| {
        let mut a = CodeAssembler::new(32).unwrap();
        f(&mut a).unwrap();
        a.assemble((DRIVER_BASE + rva) as u64).unwrap()
    };
    let flags = pe::SCN_CODE | pe::SCN_EXECUTE | pe::SCN_READ;
    // DllMain: TRUE.
    let entry = b.next_rva();
    let bytes = code(
        &|a| {
            a.mov(eax, 1)?;
            a.ret_1(12)
        },
        entry,
    );
    b.section(".t0", &bytes, flags);
    b.entry(entry);
    // DriverProc(id, driver, message, p1, p2).
    let proc_rva = b.next_rva();
    let bytes = code(
        &|a| {
            let mut skip_log = a.create_label();
            let mut checked = a.create_label();
            let mut open = a.create_label();
            let mut mci_open = a.create_label();
            let mut info = a.create_label();
            let mut status = a.create_label();
            let mut zero = a.create_label();
            let mut done = a.create_label();
            a.push(esi)?;
            a.push(edi)?;
            // The arguments are at ESP+12 on.
            a.mov(eax, dword_ptr(esp + 20))?;
            a.mov(ecx, at(LOG_COUNT))?;
            a.cmp(ecx, 16)?;
            a.jae(skip_log)?;
            a.mov(dword_ptr(ecx * 4 + (data + LOG)), eax)?;
            a.inc(at(LOG_COUNT))?;
            a.set_label(&mut skip_log)?;
            a.cmp(eax, 0x800)?;
            a.jb(checked)?;
            a.cmp(dword_ptr(esp + 12), DRIVER_ID)?;
            a.je(checked)?;
            a.mov(at(BAD_ID), 1)?;
            a.set_label(&mut checked)?;
            a.mov(esi, dword_ptr(esp + 28))?;
            a.cmp(eax, DRV_OPEN)?;
            a.je(open)?;
            a.cmp(eax, MCI_OPEN_DRIVER)?;
            a.je(mci_open)?;
            a.cmp(eax, MCI_INFO)?;
            a.je(info)?;
            a.cmp(eax, MCI_STATUS)?;
            a.je(status)?;
            a.cmp(eax, 0x800)?;
            a.jae(zero)?;
            // DRV_LOAD, DRV_ENABLE, DRV_CLOSE, DRV_DISABLE, DRV_FREE.
            a.mov(eax, 1)?;
            a.jmp(done)?;
            // MCI_OPEN_DRIVER_PARMS: the device's id, its type and the
            // driver's command table.
            a.set_label(&mut open)?;
            a.mov(eax, dword_ptr(esi))?;
            a.mov(at(DEVICE_ID), eax)?;
            a.mov(dword_ptr(esi + 12), MCI_DEVTYPE_DIGITAL_VIDEO)?;
            a.push(0)?;
            a.push(5)?;
            a.push(DRIVER_BASE as i32)?;
            a.call(import(0))?;
            a.mov(dword_ptr(esi + 8), eax)?;
            a.mov(eax, DRIVER_ID)?;
            a.jmp(done)?;
            // MCI_OPEN_PARMSW: the element's first character, and the
            // driver data for the device.
            a.set_label(&mut mci_open)?;
            a.mov(eax, dword_ptr(esi + 12))?;
            a.movzx(eax, word_ptr(eax))?;
            a.mov(at(ELEMENT), eax)?;
            a.push(0xABCD)?;
            a.push(dword_ptr(esi + 4))?;
            a.call(import(1))?;
            a.jmp(zero)?;
            a.set_label(&mut info)?;
            a.mov(edi, dword_ptr(esi + 4))?;
            a.mov(dword_ptr(edi), 0x0065_0054)?;
            a.mov(dword_ptr(edi + 4), 0x0074_0073)?;
            a.mov(word_ptr(edi + 8), 0)?;
            a.jmp(zero)?;
            a.set_label(&mut status)?;
            a.push(at(DEVICE_ID))?;
            a.call(import(2))?;
            a.mov(at(DRIVER_DATA), eax)?;
            a.mov(dword_ptr(esi + 4), 0x0123_0000 | MCI_MODE_PLAY)?;
            a.mov(eax, 0x2_0000)?;
            a.jmp(done)?;
            a.set_label(&mut zero)?;
            a.xor(eax, eax)?;
            a.set_label(&mut done)?;
            a.pop(edi)?;
            a.pop(esi)?;
            a.ret_1(20)
        },
        proc_rva,
    );
    b.section(".t1", &bytes, flags);
    b.export(Some("DriverProc"), 1, proc_rva);
    let log_rva = b.next_rva();
    let bytes = code(
        &|a| {
            a.mov(eax, data as i32)?;
            a.ret()
        },
        log_rva,
    );
    b.section(".t2", &bytes, flags);
    b.export(Some("Log"), 2, log_rva);
    b.resource(10, 5, 1033, b"table");
    b.build()
}

// The program's data: MCI_OPEN_PARMS, the element's name, MCI_INFO_PARMS
// and its buffer, MCI_STATUS_PARMS, the driver's name, "Log" and the
// messages the driver is to get.
const OPEN_PARMS: u32 = 0x00;
const ELEMENT_NAME: u32 = 0x20;
const INFO_PARMS: u32 = 0x40;
const INFO_TEXT: u32 = 0x50;
const STATUS_PARMS: u32 = 0x70;
const DRIVER_NAME: u32 = 0x80;
const LOG_NAME: u32 = 0x90;
const GENERIC_PARMS: u32 = 0x98;
const EXPECTED: u32 = 0xA0;

/// The messages the driver gets, in order: its DLL is loaded and the
/// driver opened, MCI opens it, MCI_INFO and MCI_STATUS, MCI closes it
/// and then the driver is closed, the last of its DLL.
const MESSAGES: [u32; 10] = [
    1,
    2,
    DRV_OPEN as u32,
    MCI_OPEN_DRIVER as u32,
    MCI_INFO as u32,
    MCI_STATUS as u32,
    MCI_CLOSE_DRIVER,
    4,
    5,
    6,
];

/// Opens C:\CLIP.TST, whose extension WIN.INI's [mci extensions] gives
/// the device type TestVideo, which SYSTEM.INI's [mci] gives the 16-bit
/// driver TESTV.DRV: MCI opens TESTV32.DLL. The exit code has a bit for
/// each check: the open, the id, MCI_INFO's text in ANSI, MCI_STATUS,
/// its mode without the resource, the close, the id gone after it, and
/// the driver's log (with its ids, the element in UTF-16 and its driver
/// data).
fn mci_driver_program() -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    put(&mut data, ELEMENT_NAME as usize, b"C:\\CLIP.TST");
    put(&mut data, DRIVER_NAME as usize, b"TESTV32.DLL");
    put(&mut data, LOG_NAME as usize, b"Log");
    for (i, m) in MESSAGES.iter().enumerate() {
        put(&mut data, EXPECTED as usize + 4 * i, &m.to_le_bytes());
    }
    let imports: &[(&str, &[&str])] = &[
        (
            "KERNEL32.dll",
            &["ExitProcess", "LoadLibraryA", "GetProcAddress"],
        ),
        ("WINMM.dll", &["mciSendCommandA"]),
    ];
    let send = |a: &mut CodeAssembler, l: &Layout, msg: i32, flags: i32, parms: u32| {
        a.push(l.at(parms) as i32)?;
        a.push(flags)?;
        a.push(msg)?;
        a.push(edi)?;
        a.call(l.import(3))
    };
    program(imports, &data, |a, l| {
        a.xor(ebx, ebx)?;
        // The driver's DLL stays loaded for its log to be read.
        call(a, l.import(1), &[l.at(DRIVER_NAME) as i32])?;
        a.push(l.at(LOG_NAME) as i32)?;
        a.push(eax)?;
        a.call(l.import(2))?;
        a.mov(esi, eax)?;
        a.mov(dword_ptr(l.at(OPEN_PARMS + 12)), l.at(ELEMENT_NAME) as i32)?;
        a.xor(edi, edi)?;
        send(a, l, MCI_OPEN, MCI_OPEN_ELEMENT, OPEN_PARMS)?;
        check_eax(a, 0, 0)?;
        a.mov(edi, dword_ptr(l.at(OPEN_PARMS + 4)))?;
        let mut no_id = a.create_label();
        a.test(edi, edi)?;
        a.jz(no_id)?;
        a.or(ebx, 1 << 1)?;
        a.set_label(&mut no_id)?;
        a.mov(dword_ptr(l.at(INFO_PARMS + 4)), l.at(INFO_TEXT) as i32)?;
        a.mov(dword_ptr(l.at(INFO_PARMS + 8)), 32)?;
        send(a, l, MCI_INFO, MCI_INFO_PRODUCT, INFO_PARMS)?;
        let mut info_failed = a.create_label();
        a.test(eax, eax)?;
        a.jnz(info_failed)?;
        a.cmp(dword_ptr(l.at(INFO_TEXT)), u32::from_le_bytes(*b"Test") as i32)?;
        a.jne(info_failed)?;
        a.cmp(byte_ptr(l.at(INFO_TEXT + 4)), 0)?;
        a.jne(info_failed)?;
        a.or(ebx, 1 << 2)?;
        a.set_label(&mut info_failed)?;
        a.mov(dword_ptr(l.at(STATUS_PARMS + 8)), MCI_STATUS_MODE as i32)?;
        send(a, l, MCI_STATUS, MCI_STATUS_ITEM, STATUS_PARMS)?;
        check_eax(a, 0, 3)?;
        a.mov(eax, dword_ptr(l.at(STATUS_PARMS + 4)))?;
        check_eax(a, MCI_MODE_PLAY, 4)?;
        send(a, l, MCI_CLOSE, 0, GENERIC_PARMS)?;
        check_eax(a, 0, 5)?;
        send(a, l, MCI_STATUS, MCI_STATUS_ITEM, STATUS_PARMS)?;
        check_eax(a, MCIERR_INVALID_DEVICE_ID, 6)?;
        // The driver's log.
        let mut log_wrong = a.create_label();
        a.call(esi)?;
        a.mov(esi, eax)?;
        a.cmp(dword_ptr(esi + LOG_COUNT), MESSAGES.len() as i32)?;
        a.jne(log_wrong)?;
        a.cmp(dword_ptr(esi + BAD_ID), 0)?;
        a.jne(log_wrong)?;
        a.cmp(dword_ptr(esi + ELEMENT), b'C' as i32)?;
        a.jne(log_wrong)?;
        a.cmp(dword_ptr(esi + DRIVER_DATA), 0xABCD)?;
        a.jne(log_wrong)?;
        a.lea(esi, dword_ptr(esi + LOG))?;
        a.mov(edi, l.at(EXPECTED) as i32)?;
        a.mov(ecx, MESSAGES.len() as i32)?;
        a.cld()?;
        a.repe().cmpsd()?;
        a.jne(log_wrong)?;
        a.or(ebx, 1 << 7)?;
        a.set_label(&mut log_wrong)?;
        a.push(ebx)?;
        a.call(l.import(0))?;
        a.int3()
    })
}

/// A device type the system names a driver for opens with the driver:
/// mciSendCommandA's commands go to its DriverProc, with the open's
/// strings in UTF-16 and MCI_INFO's answer back in ANSI, as Windows NT's
/// WINMM gives them to its 32-bit MCI drivers. SYSTEM.INI's [mci] names
/// Windows 9x's 16-bit driver, which the built-in Windows takes the
/// 32-bit driver of the same name for.
#[test]
fn mci_opens_a_device_with_its_installable_driver() {
    for core in CORES {
        let files = [
            ("T.EXE", mci_driver_program()),
            ("WINDOWS/SYSTEM/TESTV32.DLL", mci_driver()),
            (
                "WINDOWS/SYSTEM.INI",
                b"[mci]\r\nTestVideo=testv.drv\r\n".to_vec(),
            ),
            (
                "WINDOWS/WIN.INI",
                b"[mci extensions]\r\ntst=TestVideo\r\n".to_vec(),
            ),
        ];
        let mut cpu = machine("mci_driver", core, &files, |_| {});
        run_to_exit(&mut cpu, 2_000, |_| {});
        assert_eq!(
            cpu.errorlevel,
            0xFF,
            "{core:?}: the checks that failed: {:08b}",
            !cpu.errorlevel
        );
    }
}
