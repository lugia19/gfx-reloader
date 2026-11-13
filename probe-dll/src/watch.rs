//! Hardware write watchpoint (debug register DR0) on one byte, to find what writes it. Used for `CSMenuMan+0x10`,
//! the flag the main-menu windows' close check reads (see ..\README.md).
//!
//! F5 arms it on every thread that exists at that moment (threads created later are not covered) and disarms it on
//! the next press. A vectored exception handler catches the single-step trap the CPU raises after each write and
//! logs the writing instruction (the trap reports the address of the instruction *after* the write) with the new
//! value: the first time each (instruction, value) pair is seen, and every time the value changes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, EXCEPTION_POINTERS, GetThreadContext, SetThreadContext,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_SET_CONTEXT, THREAD_SUSPEND_RESUME,
};

use crate::log;

const CONTEXT_DEBUG_REGISTERS_AMD64: u32 = 0x0010_0010;
const STATUS_SINGLE_STEP: u32 = 0x8000_0004;
const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// GetThreadContext/SetThreadContext need a 16-byte aligned CONTEXT, but windows-sys 0.59 declares the x86-64
/// CONTEXT as plain repr(C) (8-byte alignment), so a stack CONTEXT can land misaligned and every call fails.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

static ARMED: AtomicBool = AtomicBool::new(false);
static WATCHED: AtomicUsize = AtomicUsize::new(0);
static HANDLER_ADDED: AtomicBool = AtomicBool::new(false);
/// (instruction after the write, value written) -> hits; and the last value seen.
static HITS: Mutex<Option<(HashMap<(usize, u8), u64>, Option<u8>)>> = Mutex::new(None);

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    let record = unsafe { &*(*info).ExceptionRecord };
    let context = unsafe { &mut *(*info).ContextRecord };
    if record.ExceptionCode as u32 != STATUS_SINGLE_STEP || context.Dr6 & 1 == 0 {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    context.Dr6 = 0;
    let addr = WATCHED.load(Ordering::Relaxed);
    let value = unsafe { std::ptr::read_volatile(addr as *const u8) };
    let rip = context.Rip as usize;
    let mut guard = HITS.lock().unwrap();
    let (hits, last) = guard.get_or_insert_with(|| (HashMap::new(), None));
    let count = hits.entry((rip, value)).or_insert(0);
    *count += 1;
    let changed = *last != Some(value);
    let first = *count == 1;
    let previous = *last;
    *last = Some(value);
    drop(guard);
    if first || changed {
        let thread = unsafe { GetCurrentThreadId() };
        log(&format!(
            "watch {addr:#x}: write {value} (was {}) by the instruction before {rip:#x}, thread {thread}{}{}",
            previous.map(|v| v.to_string()).unwrap_or("?".into()),
            if first { ", first time from there" } else { "" },
            if changed { ", VALUE CHANGED" } else { "" },
        ));
    }
    EXCEPTION_CONTINUE_EXECUTION
}

/// Sets (addr = Some) or clears (None) DR0 as a 1-byte write watchpoint on every other thread of the process.
fn set_on_all_threads(addr: Option<usize>) -> (usize, usize) {
    let (mut done, mut failed) = (0, 0);
    let mut first_error: Option<String> = None;
    let mut note = |stage: &str| {
        if first_error.is_none() {
            first_error = Some(format!("{stage} error {}", unsafe { GetLastError() }));
        }
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return (0, 0);
        }
        let me = GetCurrentThreadId();
        let pid = GetCurrentProcessId();
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snapshot, &mut entry) != 0;
        while ok {
            if entry.th32OwnerProcessID == pid && entry.th32ThreadID != me {
                let thread = OpenThread(
                    THREAD_GET_CONTEXT | THREAD_SET_CONTEXT | THREAD_SUSPEND_RESUME,
                    0,
                    entry.th32ThreadID,
                );
                if thread.is_null() {
                    note("OpenThread");
                    failed += 1;
                } else {
                    SuspendThread(thread);
                    let mut aligned: AlignedContext = std::mem::zeroed();
                    let context = &mut aligned.0;
                    context.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
                    if GetThreadContext(thread, context) != 0 {
                        match addr {
                            Some(a) => {
                                context.Dr0 = a as u64;
                                // L0 on, RW0 = 01 (write), LEN0 = 00 (1 byte)
                                context.Dr7 = (context.Dr7 & !(0xf << 16)) | 1 | (0b01 << 16);
                            }
                            None => {
                                context.Dr0 = 0;
                                context.Dr7 &= !(0b11 | (0xf << 16));
                            }
                        }
                        if SetThreadContext(thread, context) != 0 {
                            done += 1
                        } else {
                            note("SetThreadContext");
                            failed += 1
                        }
                    } else {
                        note("GetThreadContext");
                        failed += 1;
                    }
                    ResumeThread(thread);
                    CloseHandle(thread);
                }
            }
            ok = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    if let Some(error) = first_error {
        log(&format!("watch: first failure: {error}"));
    }
    (done, failed)
}

/// F5: arm on `addr` / disarm.
pub fn toggle(addr: usize) {
    if !HANDLER_ADDED.swap(true, Ordering::Relaxed) {
        unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
    }
    if ARMED.swap(false, Ordering::Relaxed) {
        let (done, failed) = set_on_all_threads(None);
        log(&format!("watch: disarmed on {done} threads ({failed} failed)"));
        return;
    }
    if addr == 0 {
        log("watch: nothing to watch yet");
        return;
    }
    WATCHED.store(addr, Ordering::Relaxed);
    *HITS.lock().unwrap() = Some((HashMap::new(), Some(unsafe { std::ptr::read_volatile(addr as *const u8) })));
    let (done, failed) = set_on_all_threads(Some(addr));
    ARMED.store(true, Ordering::Relaxed);
    log(&format!("watch: armed on {addr:#x} (value now {}) on {done} threads ({failed} failed)", unsafe {
        std::ptr::read_volatile(addr as *const u8)
    }));
}
