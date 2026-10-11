//! The built-in DOS's file calls on a folder mounted from the host: what
//! DIR, a game's file scan and a game's data file loads cost.
//!
//! The folders are generated at start-up under the target directory. 218
//! entries is the size of Dark Sun 2's game folder.

mod common;

use common::*;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// Folder sizes: a small game, Dark Sun 2's folder, a large one.
const SIZES: [usize; 3] = [50, 218, 1000];

/// DIR: one listing of the whole folder (what the shell's DIR asks for).
fn dir_listing(c: &mut Criterion) {
    let mut group = c.benchmark_group("dos/dir_listing");
    for count in SIZES {
        let root = game_folder(&format!("dir_listing_{count}"), count);
        let cpu = machine(&root);
        group.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| black_box(cpu.bus.disk.list_directory(r"C:\*.*", 0x16).unwrap().len()))
        });
    }
    group.finish();
}

/// FindFirst then FindNext until the end, through INT 21h, as a program's
/// own directory scan or a DIR-like tool does. Few samples and no 1000:
/// one run over 218 entries takes over a second on main.
fn find_first_next(c: &mut Criterion) {
    let mut group = c.benchmark_group("dos/find_first_next");
    group.sample_size(10);
    for count in [50, 218] {
        let root = game_folder(&format!("find_first_next_{count}"), count);
        let mut cpu = machine(&root);
        set_dta(&mut cpu);
        assert_eq!(find_all(&mut cpu, r"C:\*.*", 0x16), count);
        group.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| black_box(find_all(&mut cpu, r"C:\*.*", 0x16)))
        });
    }
    group.finish();
}

/// A game looking for one kind of file in its folder: "RGN*.GFF" over
/// Dark Sun 2's 218 entries.
fn find_pattern(c: &mut Criterion) {
    let root = game_folder("find_pattern", 218);
    let mut cpu = machine(&root);
    set_dta(&mut cpu);
    let expected = game_names(218)
        .iter()
        .filter(|name| name.starts_with("RGN") && name.ends_with(".GFF"))
        .count();
    assert_eq!(find_all(&mut cpu, r"C:\RGN*.GFF", 0), expected);
    c.bench_function("dos/find_pattern/218", |b| {
        b.iter(|| black_box(find_all(&mut cpu, r"C:\RGN*.GFF", 0)))
    });
}

/// Opening and closing a data file three folders deep, each folder holding
/// a game folder's worth of entries: C:\GAMES\DARKSUN\DATA\RGN0D2.GFF.
fn open_deep(c: &mut Criterion) {
    let root = scratch("open_deep");
    let games = root.join("GAMES");
    write_files(&games, &game_names(50));
    let game = games.join("DARKSUN");
    write_files(&game, &game_names(218));
    let data = game.join("DATA");
    write_files(&data, &game_names(218));
    let mut cpu = machine(&root);
    let path = r"C:\GAMES\DARKSUN\DATA\RGN0D2.GFF";
    open_close(&mut cpu, path);
    c.bench_function("dos/open_close_deep", |b| b.iter(|| open_close(&mut cpu, path)));
}

/// A machine with a fresh 40 MB FAT16 hard disk image as D:.
fn fat_machine(name: &str) -> rust_dos::cpu::Cpu {
    use rust_dos::makeimg::{self, ImageSpec};
    let dir = scratch(name);
    let image = dir.join("hdd.img");
    let plan = makeimg::plan(&ImageSpec { preset: makeimg::preset("hd_40mb"), ..Default::default() }).unwrap();
    makeimg::write(&image, &plan, false).unwrap();
    let mut cpu = machine(&dir);
    let opts = rust_dos::disk::MountOptions { kind: rust_dos::disk::DriveKind::HardDisk, ..Default::default() };
    cpu.bus.mount_drive(3, &image, opts, true).unwrap();
    cpu
}

/// Reading a 2 MB file on a FAT disk image start to end, in reads of 512
/// bytes and of 16 KB, as games load their data.
fn fat_read(c: &mut Criterion) {
    let mut cpu = fat_machine("fat_read");
    const LEN: usize = 8 << 20;
    write_file(&mut cpu, r"D:\DATA.BIN", LEN, 32 << 10);
    let mut group = c.benchmark_group("dos/fat_read_8mb");
    group.sample_size(10);
    for chunk in [512u16, 16 << 10] {
        assert_eq!(read_file(&mut cpu, r"D:\DATA.BIN", chunk), LEN);
        group.bench_function(BenchmarkId::from_parameter(chunk), |b| {
            b.iter(|| black_box(read_file(&mut cpu, r"D:\DATA.BIN", chunk)))
        });
    }
    group.finish();
}

/// Writing an 8 MB file on a FAT disk image in writes of 512 bytes and of
/// 16 KB, as a game saves (the file is made anew each time).
fn fat_write(c: &mut Criterion) {
    let mut cpu = fat_machine("fat_write");
    let mut group = c.benchmark_group("dos/fat_write_8mb");
    group.sample_size(10);
    for chunk in [512u16, 16 << 10] {
        group.bench_function(BenchmarkId::from_parameter(chunk), |b| {
            b.iter(|| {
                write_file(&mut cpu, r"D:\SAVE.DAT", 8 << 20, chunk);
                set_dsdx_string(&mut cpu, r"D:\SAVE.DAT");
                int21(&mut cpu, 0x41);
            })
        });
    }
    group.finish();
}

criterion_group!(benches, dir_listing, find_first_next, find_pattern, open_deep, fat_read, fat_write);
criterion_main!(benches);
