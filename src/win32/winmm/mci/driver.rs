//! MCI's installable drivers: a device type that none of the built-in
//! devices is, but that the system names a driver for, opens the driver
//! as Windows NT's WINMM and Wine's do. The driver's DLL is opened as an
//! installable driver (winmm's driver.rs) with an MCI_OPEN_DRIVER_PARMS
//! as DRV_OPEN's second parameter, in which it gives its device type and
//! command table; it is sent MCI_OPEN_DRIVER with the open's parameters,
//! every command of mciSendCommandA after that, and MCI_CLOSE_DRIVER
//! before it is closed. The driver does the commands' work itself,
//! waits included, and posts their notifications.
//!
//! The driver of a type is the one the registry's
//! `HKLM\Software\Microsoft\Windows NT\CurrentVersion\MCI32` key or
//! SYSTEM.INI's [mci32] section names, where Windows NT lists its 32-bit
//! MCI drivers, or else the one SYSTEM.INI's [mci] section names, where
//! Windows 95 and 98 list theirs. Those are 16-bit drivers, which
//! Windows 9x's 16-bit MMSYSTEM loads. A 16-bit driver for a 32-bit one
//! is a thunk to it: ActiveMovie's MCIQTZ.DRV ("MCIQTZ Thunk 16") runs
//! MCIQTZ32.DLL's driver in a process of its own (`rundll32
//! mciqtz32.dll,MCIEntry32`). The built-in Windows has no 16-bit
//! MMSYSTEM to run such a driver, so for a `.drv` named in [mci] it
//! opens the 32-bit driver of the same name with "32" after it
//! (MCIQTZ32.DLL), in the program's process as Windows NT does, when the
//! system directory has it.
//!
//! 32-bit drivers take Unicode: the strings in the parameters of
//! MCI_OPEN, MCI_INFO, MCI_LOAD, MCI_SAVE, MCI_WINDOW and MCI_ESCAPE are
//! given to the driver in UTF-16, and MCI_INFO's answer comes back in
//! ANSI, as Windows NT's mciSendCommandA does.

use super::*;
use crate::win32::call;
use crate::win32::registry;

/// The driver messages MCI sends a driver's DriverProc, besides its
/// commands.
const MCI_OPEN_DRIVER: u32 = 0x0801;
const MCI_CLOSE_DRIVER: u32 = 0x0802;

/// The commands with strings in their parameters, besides MCI_OPEN and
/// MCI_INFO.
const MCI_ESCAPE: u32 = 0x0805;
const MCI_WINDOW: u32 = 0x0841;

/// The flags that say a string is there: MCI_ANIM_WINDOW_TEXT (and
/// MCI_DGV_WINDOW_TEXT), MCI_LOAD_FILE and MCI_SAVE_FILE, and
/// MCI_VD_ESCAPE_STRING.
const MCI_WINDOW_TEXT: u32 = 0x0008_0000;
const MCI_FILE: u32 = 0x0100;
const MCI_ESCAPE_STRING: u32 = 0x0100;

/// The flags of an open whose MCI_OPEN_PARMS goes on with a style and a
/// parent window (MCI_ANIM_OPEN_WS, MCI_ANIM_OPEN_PARENT and their
/// MCI_DGV_ and MCI_OVLY_ twins), or a wave device's buffer seconds
/// (MCI_WAVE_OPEN_BUFFER, as MCI_ANIM_OPEN_WS).
const MCI_OPEN_WORD_5: u32 = 0x0001_0000;
const MCI_OPEN_WORD_6: u32 = 0x0002_0000;

/// What a driver's mciLoadCommandResource gets when there is no table,
/// and what it sets as MCI_OPEN_DRIVER_PARMS' table without one.
pub const MCI_NO_COMMAND_TABLE: u32 = 0xFFFF;

/// The high word of a command's result that says MCI_STATUS's or
/// MCI_GETDEVCAPS' dwReturn holds a constant in its low word and the
/// constant's string resource in its high word.
const MCI_RESOURCE_RETURNED: u32 = 0x0002_0000;

/// The type of a driver's custom command table (mciLoadCommandResource).
const RT_RCDATA: u32 = 10;

/// The longest string a command's parameters give, in characters.
const MAX_STRING: usize = 260;

/// What a call to a driver's DriverProc was for (`returned`): its
/// MCI_OPEN_DRIVER, a command, or its MCI_CLOSE_DRIVER.
const OP_OPEN: u32 = 0;
const OP_COMMAND: u32 = 1;
const OP_CLOSE: u32 = 2;

/// A command table a driver loaded (mciLoadCommandResource): its id,
/// the process and module it is in and where its data is.
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub id: u32,
    pub pid: u32,
    pub module: u32,
    pub at: u32,
    pub size: u32,
    pub kind: u32,
}

crate::state_fields!(Table {
    id,
    pid,
    module,
    at,
    size,
    kind
});

/// The registry key of Windows NT's 32-bit MCI drivers, and of the
/// device types of files' extensions.
const MCI32_KEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\MCI32";
const EXTENSIONS_KEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\MCI Extensions";

/// The driver the system names for the device type `name`: its DLL and
/// what follows the DLL's name, the driver's parameters. None when it
/// names none.
pub fn driver_file(cpu: &mut Cpu, name: &str) -> Option<(String, String)> {
    if name.is_empty() {
        return None;
    }
    let nt = registry::string_value(cpu, registry::HKEY_LOCAL_MACHINE, MCI32_KEY, name)
        .or_else(|| profile(cpu, b"SYSTEM.INI", b"mci32", name));
    let (file, params) = match nt {
        Some(value) => split(&value),
        None => {
            let (file, params) = split(&profile(cpu, b"SYSTEM.INI", b"mci", name)?);
            (thunked(cpu, &file).unwrap_or(file), params)
        }
    };
    (!file.is_empty()).then_some((file, params))
}

/// The 32-bit driver a 16-bit `.drv` thunks to, by its name: `file`'s
/// name with "32.dll" for ".drv", if the system directory has it.
fn thunked(cpu: &mut Cpu, file: &str) -> Option<String> {
    let name = file.rsplit(['\\', '/', ':']).next().unwrap_or(file);
    let (stem, ext) = name.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("drv") {
        return None;
    }
    let dll = format!("{}32.dll", stem);
    let path = format!("{}\\{}", crate::win32::loader::SYSTEM_DIR, dll);
    cpu.bus.disk.is_file(&path).then_some(dll)
}

/// A profile's value for the system's use, as text.
fn profile(cpu: &mut Cpu, file: &[u8], section: &[u8], key: &str) -> Option<String> {
    let value = crate::win32::kernel32::profile_value(cpu, file, section, key.as_bytes())?;
    Some(crate::dosstr::from_bytes(&value))
}

/// A driver's line: its file, and its parameters after a space.
fn split(value: &str) -> (String, String) {
    let value = value.trim();
    match value.split_once(' ') {
        Some((file, params)) => (file.to_string(), params.trim().to_string()),
        None => (value.to_string(), String::new()),
    }
}

/// The device type of a file's extension that the system names: WIN.INI's
/// [mci extensions], as Windows 95 and 98 have them, or Windows NT's
/// registry key. None for none.
pub fn type_of_extension(cpu: &mut Cpu, ext: &str) -> Option<String> {
    if ext.is_empty() {
        return None;
    }
    profile(cpu, b"WIN.INI", b"mci extensions", ext)
        .or_else(|| {
            registry::string_value(cpu, registry::HKEY_LOCAL_MACHINE, EXTENSIONS_KEY, ext)
        })
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Memory of the running process's heap, zeroed: its address, or 0.
fn alloc(cpu: &mut Cpu, size: u32) -> u32 {
    let at = crate::win32::with_process(cpu, |p, bus| p.heap.alloc(&mut p.memory, bus, size));
    match at {
        Some(at) => {
            cpu.bus.guest_fill(at, size as usize, 0);
            at
        }
        None => 0,
    }
}

fn free(cpu: &mut Cpu, at: u32) {
    if at != 0 {
        crate::win32::with_process(cpu, |p, _| p.heap.free(at));
    }
}

/// `text` in UTF-16 with its 0.
fn wide(text: &str) -> Vec<u8> {
    let units = crate::win32::kernel32::ansi_to_wide(&crate::dosstr::to_bytes(text));
    units
        .iter()
        .chain(&[0])
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

/// The UTF-16 string at `at`, at most `max` characters, as ANSI.
fn read_wide(cpu: &mut Cpu, at: u32, max: usize) -> String {
    let mut units = Vec::new();
    for i in 0..max as u32 {
        let unit = cpu.bus.guest_read_16(at.wrapping_add(2 * i));
        if unit == 0 || cpu.bus.guest_faulted() {
            break;
        }
        units.push(unit);
    }
    crate::dosstr::from_bytes(&crate::win32::kernel32::wide_to_ansi(&units))
}

/// A block of the heap that a driver's call is given: the structures
/// first, then the strings they point to.
struct Block {
    words: Vec<u32>,
    strings: Vec<(usize, Vec<u8>)>,
}

impl Block {
    /// Put the block in the heap: its address, or 0 when memory is out.
    fn place(&self, cpu: &mut Cpu) -> u32 {
        let head = 4 * self.words.len() as u32;
        let size = head + self.strings.iter().map(|(_, s)| s.len() as u32).sum::<u32>();
        let at = alloc(cpu, size.max(4));
        if at == 0 {
            return 0;
        }
        let mut words = self.words.clone();
        let mut next = at + head;
        for (word, bytes) in &self.strings {
            words[*word] = next;
            cpu.bus.guest_write_bytes(next, bytes);
            next += bytes.len() as u32;
        }
        for (i, &word) in words.iter().enumerate() {
            cpu.bus.guest_write_32(at + 4 * i as u32, word);
        }
        at
    }
}

/// The ANSI string at `at` of the program's parameters, in UTF-16; None
/// when its page isn't there.
fn wide_at(cpu: &mut Cpu, at: u32) -> Option<Vec<u8>> {
    Some(wide(&read_string(cpu, at, MAX_STRING)?))
}

/// Open the device type `name` with its driver, for the open of
/// `flags` and `p` (`at` the program's MCI_OPEN_PARMS, or 0): the
/// device's id goes to `at` when its driver has opened it. MCI_OPEN's
/// result, or None while the driver's DLL loads and its DriverProc
/// runs, which give it to the program.
pub fn open(
    cpu: &mut Cpu,
    flags: u32,
    p: &Parms,
    name: &str,
    element: String,
    auto: bool,
    at: u32,
) -> Option<u32> {
    let Some((file, params)) = driver_file(cpu, name) else {
        return Some(MCIERR_INVALID_DEVICE_NAME);
    };
    let alias = if flags & MCI_OPEN_ALIAS != 0 {
        p.alias.clone()
    } else if !element.is_empty() {
        element.clone()
    } else {
        name.to_string()
    };
    if find_name(cpu, &alias).is_some() {
        return Some(if flags & MCI_OPEN_ALIAS != 0 {
            MCIERR_DUPLICATE_ALIAS
        } else {
            MCIERR_DEVICE_OPEN
        });
    }
    let mut id = 1;
    while cpu.bus.win32.winmm.mci.devices.iter().any(|d| d.id == id) {
        id += 1;
    }
    // The open's style and parent window, or buffer seconds, which the
    // program's structure has after the alias when its flags say so.
    let mut extra = [0u32; 2];
    for (i, flag) in [MCI_OPEN_WORD_5, MCI_OPEN_WORD_6].into_iter().enumerate() {
        let word = at.wrapping_add(20 + 4 * i as u32);
        if at != 0 && flags & flag != 0 {
            if !cpu.bus.guest_probe(word, 4, false) {
                return None;
            }
            extra[i] = cpu.bus.guest_read_32(word);
        }
    }
    // MCI_OPEN_DRIVER_PARMS (id, parameters, command table, type), then
    // MCI_OPEN_PARMSW and the rest of the open's parameters.
    let mut block = Block {
        words: vec![id, 0, MCI_NO_COMMAND_TABLE, 0],
        strings: Vec::new(),
    };
    if !params.is_empty() {
        block.strings.push((1, wide(&params)));
    }
    let type_word = if flags & MCI_OPEN_TYPE_ID != 0 {
        p.open_type_id
    } else {
        0
    };
    let element_word = if flags & MCI_OPEN_ELEMENT_ID != 0 && at != 0 {
        cpu.bus.guest_read_32(at + 12)
    } else {
        0
    };
    block.words.extend([
        p.callback,
        id,
        type_word,
        element_word,
        0,
        extra[0],
        extra[1],
    ]);
    if flags & MCI_OPEN_TYPE_ID == 0 {
        block.strings.push((6, wide(name)));
    }
    if flags & MCI_OPEN_ELEMENT_ID == 0 && !element.is_empty() {
        block.strings.push((7, wide(&element)));
    }
    if flags & MCI_OPEN_ALIAS != 0 {
        block.strings.push((8, wide(&p.alias)));
    }
    let block = block.place(cpu);
    if block == 0 {
        return Some(MCIERR_OUT_OF_MEMORY);
    }
    let pid = crate::win32::user32::current_pid(cpu);
    cpu.bus.win32.winmm.mci.devices.push(Device {
        id,
        pid,
        name: alias,
        element,
        auto,
        time_format: MCI_FORMAT_MILLISECONDS,
        opening: true,
        type_name: name.to_string(),
        block,
        open_at: at,
        open_flags: flags,
        ..Device::default()
    });
    cpu.bus.log_string(&format!(
        "[W32] MCI: {} opens with the driver {}",
        name, file
    ));
    let handle = crate::win32::winmm::driver::open_for_mci(cpu, &file, block, id);
    if let Some(i) = index(cpu, id) {
        device(cpu, i).driver = handle;
    }
    None
}

/// The place of the device `id` of the running process, whether or not
/// its driver has opened it.
fn index(cpu: &Cpu, id: u32) -> Option<usize> {
    let pid = crate::win32::user32::current_pid(cpu);
    cpu.bus
        .win32
        .winmm
        .mci
        .devices
        .iter()
        .position(|d| d.id == id && d.pid == pid)
}

/// The device `id`'s driver couldn't be opened: the device goes.
pub fn open_failed(cpu: &mut Cpu, id: u32) {
    if let Some(i) = index(cpu, id) {
        let block = device(cpu, i).block;
        free(cpu, block);
        mci(cpu).devices.remove(i);
    }
}

/// The device `id`'s driver `handle` is open: it gives its type and
/// command table, and is sent MCI_OPEN_DRIVER with the open's
/// parameters. `resume` is where mciSendCommandA returns to.
pub fn opened(cpu: &mut Cpu, id: u32, handle: u32, resume: (u32, u32)) {
    let Some(i) = index(cpu, id) else {
        crate::win32::winmm::driver::close_for_mci(cpu, handle, MCIERR_INTERNAL, resume);
        return;
    };
    let block = device(cpu, i).block;
    let table = cpu.bus.guest_read_32(block + 8) & 0xFFFF;
    let kind = cpu.bus.guest_read_32(block + 12) & 0xFFFF;
    let (flags, at) = {
        let d = device(cpu, i);
        d.driver = handle;
        d.table = table;
        d.kind = kind;
        (d.open_flags, d.open_at)
    };
    let data = vec![OP_OPEN, id, at, flags];
    crate::win32::winmm::driver::send_for_mci(
        cpu,
        handle,
        MCI_OPEN_DRIVER,
        flags,
        block + 16,
        resume,
        data,
    );
}

/// A command for the open device `i` of a driver: what the driver
/// returns goes to `resume` (`returned`). `at` is the program's
/// parameters.
pub fn command(
    cpu: &mut Cpu,
    i: usize,
    msg: u32,
    flags: u32,
    at: u32,
    resume: (u32, u32),
) -> Option<u32> {
    let (id, handle) = {
        let d = device(cpu, i);
        (d.id, d.driver)
    };
    if msg == MCI_CLOSE {
        let data = vec![OP_CLOSE, id, flags, at];
        crate::win32::winmm::driver::send_for_mci(
            cpu,
            handle,
            MCI_CLOSE_DRIVER,
            flags,
            at,
            resume,
            data,
        );
        return None;
    }
    let (parms, block) = match unicode(cpu, msg, flags, at)? {
        Ok(Some(block)) => (block, block),
        Ok(None) => (at, 0),
        Err(error) => return Some(error),
    };
    let data = vec![OP_COMMAND, id, msg, flags, at, block];
    crate::win32::winmm::driver::send_for_mci(
        cpu,
        handle,
        msg,
        flags,
        parms,
        resume,
        data,
    );
    None
}

/// The command's parameters as the driver takes them, when they have
/// strings: a block of the heap with them in UTF-16 (Ok(None) when they
/// have none, or an error). None when a page isn't there.
fn unicode(cpu: &mut Cpu, msg: u32, flags: u32, at: u32) -> Option<Result<Option<u32>, u32>> {
    if at == 0 {
        return Some(Ok(None));
    }
    // The structure's words, and which of them is a string.
    let (len, string) = match msg {
        MCI_INFO => (3, None),
        MCI_LOAD | MCI_SAVE if flags & MCI_FILE != 0 => (2, Some(1)),
        MCI_WINDOW if flags & MCI_WINDOW_TEXT != 0 => (4, Some(3)),
        MCI_ESCAPE if flags & MCI_ESCAPE_STRING != 0 => (2, Some(1)),
        _ => return Some(Ok(None)),
    };
    if !cpu.bus.guest_probe(at, 4 * len, false) {
        return None;
    }
    let words: Vec<u32> = (0..len as u32)
        .map(|k| cpu.bus.guest_read_32(at + 4 * k))
        .collect();
    let mut block = Block {
        words: words.clone(),
        strings: Vec::new(),
    };
    if msg == MCI_INFO {
        // The answer's buffer, of the same number of characters.
        let room = words[2].min(0x1_0000);
        block.strings.push((1, vec![0; 2 * room.max(1) as usize]));
    } else if let Some(k) = string
        && words[k] != 0
    {
        let text = wide_at(cpu, words[k])?;
        block.strings.push((k, text));
    }
    let placed = block.place(cpu);
    Some(if placed == 0 {
        Err(MCIERR_OUT_OF_MEMORY)
    } else {
        Ok(Some(placed))
    })
}

/// What mciSendCommandA returns of what a driver returned: its low word,
/// the error. A constant MCI_STATUS or MCI_GETDEVCAPS gives with its
/// string resource (MCI_RESOURCE_RETURNED) loses the resource, as
/// Windows does.
fn result(cpu: &mut Cpu, value: u32, msg: u32, at: u32) -> u32 {
    let error = value & 0xFFFF;
    if error == 0
        && matches!(msg, MCI_STATUS | MCI_GETDEVCAPS)
        && value & MCI_RESOURCE_RETURNED != 0
        && at != 0
        && cpu.bus.guest_probe(at + 4, 4, true)
    {
        let r = cpu.bus.guest_read_32(at + 4);
        cpu.bus.guest_write_32(at + 4, r & 0xFFFF);
    }
    error
}

/// A driver's DriverProc returned `value` for the call `data` says
/// (`OP_*`, the device's id, what follows). `handle` is its driver.
pub fn returned(cpu: &mut Cpu, handle: u32, value: u32, resume: (u32, u32), data: &[u32]) {
    match *data {
        [OP_OPEN, id, at, _flags] => {
            let error = value & 0xFFFF;
            let Some(i) = index(cpu, id) else {
                let error = if error == 0 { MCIERR_INTERNAL } else { error };
                crate::win32::winmm::driver::close_for_mci(cpu, handle, error, resume);
                return;
            };
            let block = device(cpu, i).block;
            free(cpu, block);
            device(cpu, i).block = 0;
            if error != 0 {
                mci(cpu).devices.remove(i);
                crate::win32::winmm::driver::close_for_mci(cpu, handle, error, resume);
                return;
            }
            device(cpu, i).opening = false;
            if at != 0 && cpu.bus.guest_probe(at + 4, 4, true) {
                cpu.bus.guest_write_32(at + 4, id);
            }
            call::finish(cpu, resume, 0);
        }
        [OP_COMMAND, _id, msg, flags, at, block] => {
            if block != 0 {
                if msg == MCI_INFO && value & 0xFFFF == 0 {
                    let (to, room) = (cpu.bus.guest_read_32(at + 4), cpu.bus.guest_read_32(at + 8));
                    let answer = cpu.bus.guest_read_32(block + 4);
                    let text = read_wide(cpu, answer, room as usize);
                    if to != 0 && write_text(cpu, to, room, &text).is_none() {
                        free(cpu, block);
                        call::finish(cpu, resume, MCIERR_INTERNAL);
                        return;
                    }
                }
                free(cpu, block);
            }
            let _ = flags;
            let error = result(cpu, value, msg, at);
            call::finish(cpu, resume, error);
        }
        [OP_CLOSE, id, _flags, _at] => {
            if let Some(i) = index(cpu, id) {
                mci(cpu).devices.remove(i);
            }
            crate::win32::winmm::driver::close_for_mci(cpu, handle, value & 0xFFFF, resume);
        }
        _ => call::finish(cpu, resume, MCIERR_INTERNAL),
    }
}

/// The first open device of a driver of the running process, for "close
/// all": its place.
pub fn first(cpu: &Cpu) -> Option<usize> {
    let pid = crate::win32::user32::current_pid(cpu);
    cpu.bus
        .win32
        .winmm
        .mci
        .devices
        .iter()
        .position(|d| d.pid == pid && d.driver != 0 && !d.opening)
}

/// The device of the running process whose driver data a driver asks
/// for (mciGetDriverData, mciSetDriverData): one it is opening counts.
fn of_driver(cpu: &mut Cpu, id: u32) -> Option<&mut Device> {
    let i = index(cpu, id)?;
    let d = device(cpu, i);
    (d.driver != 0 || d.opening).then_some(d)
}

/// mciGetDriverData(id): what the device's driver set, or 0.
pub fn mci_get_driver_data(cpu: &mut Cpu, args: &[u32]) -> Option<u32> {
    Some(of_driver(cpu, args[0]).map_or(0, |d| d.private))
}

/// mciSetDriverData(id, data): TRUE, or FALSE for a device of no driver.
pub fn mci_set_driver_data(cpu: &mut Cpu, args: &[u32]) -> Option<u32> {
    Some(match of_driver(cpu, args[0]) {
        Some(d) => {
            d.private = args[1];
            TRUE
        }
        None => FALSE,
    })
}

/// mciDriverYield(id): 0, the wait goes on. A yield procedure the
/// program set (mciSetYieldProc) isn't called, as no built-in one is.
pub fn mci_driver_yield(_: &mut Cpu, _: &[u32]) -> Option<u32> {
    Some(0)
}

/// mciDriverNotify(window, id, status): MM_MCINOTIFY is posted to the
/// window. TRUE when there is one.
pub fn mci_driver_notify(cpu: &mut Cpu, args: &[u32]) -> Option<u32> {
    let &[hwnd, id, status] = args else {
        unreachable!()
    };
    if hwnd == 0 {
        return Some(FALSE);
    }
    post(cpu, hwnd, status, id);
    Some(TRUE)
}

/// mciLoadCommandResource(module, name, type): the command table that is
/// the RCDATA resource `name` (UTF-16) of the driver's module: its id,
/// or MCI_NO_COMMAND_TABLE when the module has none of that name.
pub fn mci_load_command_resource(cpu: &mut Cpu, args: &[u32]) -> Option<u32> {
    let &[module, name, kind] = args else {
        unreachable!()
    };
    if name == 0 {
        return Some(MCI_NO_COMMAND_TABLE);
    }
    let name = if name < 0x1_0000 {
        format!("#{}", name)
    } else {
        read_wide(cpu, name, MAX_STRING)
    };
    let Some((at, size)) = crate::win32::kernel32::resource_named(cpu, module, RT_RCDATA, &name)
    else {
        return Some(MCI_NO_COMMAND_TABLE);
    };
    let pid = crate::win32::user32::current_pid(cpu);
    let tables = &mut cpu.bus.win32.winmm.mci.tables;
    let mut id = 1;
    while tables.iter().any(|t| t.id == id) {
        id += 1;
    }
    tables.push(Table {
        id,
        pid,
        module,
        at,
        size,
        kind,
    });
    Some(id)
}

/// mciFreeCommandResource(table): TRUE, or FALSE for no table.
pub fn mci_free_command_resource(cpu: &mut Cpu, args: &[u32]) -> Option<u32> {
    let pid = crate::win32::user32::current_pid(cpu);
    let tables = &mut cpu.bus.win32.winmm.mci.tables;
    let before = tables.len();
    tables.retain(|t| !(t.id == args[0] && t.pid == pid));
    Some((tables.len() != before) as u32)
}

/// A process ended: its tables went with its modules.
pub fn process_ended(cpu: &mut Cpu, pid: u32) {
    cpu.bus.win32.winmm.mci.tables.retain(|t| t.pid != pid);
}
