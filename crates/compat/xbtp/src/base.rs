use crate::sys::{hr_from_win32, GetLastError, GetModuleFileNameW, GetProcAddress, LoadLibraryExW, OutputDebugStringA, SysAllocString, SysFreeString, E_NOTIMPL, HANDLE, HRESULT};
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Once, OnceLock};

pub static INIT_HOOK: OnceLock<fn() -> i32> = OnceLock::new();

pub type XtfLogMessageFn = unsafe extern "C" fn(u32, *mut c_void, *const u8, *const u8, i32, *const u8, *mut u8, u32) -> i32;
pub type XtfLogGetMainLevelFn = unsafe extern "C" fn() -> i32;
pub type XtfLogSetMainLevelFn = unsafe extern "C" fn(u32) -> HRESULT;
pub type XtfLogSetMainLevelAndFileFn = unsafe extern "C" fn(u32, *const u16) -> HRESULT;
pub type KdPrintFn = unsafe extern "system" fn(*const u8, u32, u32, *const u8, *mut u8) -> u32;

pub const XTF_LOG_MESSAGE: usize = 0;
pub const XTF_LOG_MESSAGE_CONTEXT: usize = 1;
pub const XTF_LOG_ASSERT_MESSAGE: usize = 2;
pub const XTF_LOG_GET_MAIN_LEVEL: usize = 3;
pub const XTF_LOG_SET_MAIN_LEVEL: usize = 4;
pub const XTF_LOG_SET_MAIN_LEVEL_AND_FILE: usize = 5;
pub const XTF_LOG_GET_TEXT_COLORS: usize = 6;
pub const XTF_LOG_GET_DEBUG_TYPE: usize = 7;

const PROC_NAMES: [&[u8]; 8] = [
    b"XtfLogMessage\0",
    b"XtfLogMessageContext\0",
    b"XtfLogAssertMessage\0",
    b"XtfLogGetMainLevel\0",
    b"XtfLogSetMainLevel\0",
    b"XtfLogSetMainLevelAndFile\0",
    b"XtfLogGetTextColors\0",
    b"XtfLogGetDebugType\0",
];

pub static LOG_TABLE: [AtomicPtr<c_void>; 8] = [
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
    AtomicPtr::new(core::ptr::null_mut()),
];

pub static KD_PRINT: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

static LOADED: Once = Once::new();
static WSA_INIT: OnceLock<HRESULT> = OnceLock::new();

#[repr(C)]
pub struct LogContext {
    pub a: u64,
    pub b: u64,
    pub c: i32,
    pub d: u32,
}

pub struct StaticContext(UnsafeCell<LogContext>);
unsafe impl Sync for StaticContext {}

impl StaticContext {
    pub fn ptr(&self) -> *mut c_void {
        self.0.get() as *mut c_void
    }
}

pub static LOG_CONTEXT: StaticContext = StaticContext(UnsafeCell::new(LogContext { a: 0, b: 0, c: -1, d: 0 }));
pub static LOADER_CONTEXT: StaticContext = StaticContext(UnsafeCell::new(LogContext { a: 0, b: 0, c: -1, d: 0 }));

#[link(name = "ntdll")]
extern "system" {
    fn vDbgPrintExWithPrefix(prefix: *const u8, component: u32, level: u32, fmt: *const u8, args: *mut u8) -> u32;
}

#[repr(C)]
struct WSADATA {
    w_version: u16,
    w_high_version: u16,
    i_max_sockets: u16,
    i_max_udp_dg: u16,
    lp_vendor_info: *mut u8,
    sz_description: [u8; 257],
    sz_system_status: [u8; 129],
}

#[link(name = "ws2_32")]
extern "system" {
    fn WSAStartup(version: u16, data: *mut WSADATA) -> i32;
}

const LOAD_ERROR: &[u8] = b"ERROR: Could not load XtfBaseServices.dll. Logging, error system and other functions will be disabled.\0";
const PROC_ERROR: &[u8] = b"Error %d GetProcAddress(0x%08X, %hs)\0";
const PROC_FILE: &[u8] = b"xbox\\devex\\xtf\\lib\\xtfbase\\XtfBaseDll.h\0";
const PROC_FUNC: &[u8] = b"xtfbase::XtfGetProcAddress\0";

fn entry(i: usize) -> *mut c_void {
    ensure_loaded();
    LOG_TABLE[i].load(Ordering::Acquire)
}

unsafe fn load_table() {
    let mut path = [0u16; 260];
    GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), path.len() as u32);
    let module_path = SysAllocString(path.as_ptr());
    let empty = [0u16; 1];
    let combined = SysAllocString(empty.as_ptr());
    let mut module: HANDLE = LoadLibraryExW(if combined.is_null() { empty.as_ptr() } else { combined }, core::ptr::null_mut(), 0);
    if module.is_null() {
        let name = crate::sys::wide("XtfBaseServices.dll");
        module = LoadLibraryExW(name.as_ptr(), core::ptr::null_mut(), 0);
    }
    SysFreeString(combined);
    SysFreeString(module_path);
    if module.is_null() {
        OutputDebugStringA(LOAD_ERROR.as_ptr());
        return;
    }
    for (i, name) in PROC_NAMES.iter().enumerate() {
        let p = GetProcAddress(module, name.as_ptr());
        if p.is_null() {
            let err = GetLastError();
            if main_level() > 0 {
                let args = [err as usize, module as usize, name.as_ptr() as usize];
                log_message(2, LOADER_CONTEXT.ptr(), PROC_FILE.as_ptr(), PROC_FUNC.as_ptr(), 0x28, PROC_ERROR.as_ptr(), args.as_ptr() as *mut u8, 1);
            }
        }
        LOG_TABLE[i].store(p, Ordering::Release);
    }
}

pub fn ensure_loaded() {
    LOADED.call_once(|| unsafe { load_table() });
}

pub unsafe fn log_message(level: u32, ctx: *mut c_void, file: *const u8, func: *const u8, line: i32, fmt: *const u8, args: *mut u8, flags: u32) {
    let p = LOG_TABLE[XTF_LOG_MESSAGE].load(Ordering::Acquire);
    if !p.is_null() {
        let f: XtfLogMessageFn = core::mem::transmute(p);
        f(level, ctx, file, func, line, fmt, args, flags);
        return;
    }
    if flags & 2 != 0 {
        OutputDebugStringA(b"Can't output customer string:\0".as_ptr());
        if !fmt.is_null() {
            OutputDebugStringA(fmt);
        }
        OutputDebugStringA(b"\r\n\0".as_ptr());
    }
}

pub fn main_level() -> i32 {
    let p = LOG_TABLE[XTF_LOG_GET_MAIN_LEVEL].load(Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    unsafe {
        let f: XtfLogGetMainLevelFn = core::mem::transmute(p);
        f()
    }
}

fn gate() -> i32 {
    ensure_loaded();
    main_level()
}

#[export_name = "?InternalLog@@YAXGPEBDPEAD@Z"]
pub unsafe extern "C" fn InternalLog(level: u16, fmt: *const u8, args: *mut u8) {
    ensure_loaded();
    log_message(level as u32, LOG_CONTEXT.ptr(), core::ptr::null(), core::ptr::null(), 0, fmt, args, 0x11);
}

#[export_name = "?LogErrorV@@YAXPEBDPEAD@Z"]
pub unsafe extern "C" fn LogErrorV(fmt: *const u8, args: *mut u8) {
    if gate() >= 1 {
        InternalLog(2, fmt, args);
    }
}

#[export_name = "?LogWarningV@@YAXPEBDPEAD@Z"]
pub unsafe extern "C" fn LogWarningV(fmt: *const u8, args: *mut u8) {
    if gate() >= 3 {
        InternalLog(3, fmt, args);
    }
}

#[export_name = "?LogInfoV@@YAXPEBDPEAD@Z"]
pub unsafe extern "C" fn LogInfoV(fmt: *const u8, args: *mut u8) {
    if gate() >= 4 {
        InternalLog(4, fmt, args);
    }
}

#[export_name = "?KdDbgPrintV@@YAXPEBD0PEAD@Z"]
pub unsafe extern "C" fn KdDbgPrintV(prefix: *const u8, fmt: *const u8, args: *mut u8) {
    let p = KD_PRINT.load(Ordering::Acquire);
    if p.is_null() {
        vDbgPrintExWithPrefix(prefix, 0, 0, fmt, args);
    } else {
        let f: KdPrintFn = core::mem::transmute(p);
        f(prefix, 0, 0, fmt, args);
    }
}

core::arch::global_asm!(
    ".text",
    ".globl \"?LogError@@YAXPEBDZZ\"",
    ".p2align 4",
    "\"?LogError@@YAXPEBDZZ\":",
    ".seh_proc \"?LogError@@YAXPEBDZZ\"",
    "mov qword ptr [rsp + 8], rcx",
    "mov qword ptr [rsp + 16], rdx",
    "mov qword ptr [rsp + 24], r8",
    "mov qword ptr [rsp + 32], r9",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "lea rdx, [rsp + 56]",
    "call \"?LogErrorV@@YAXPEBDPEAD@Z\"",
    "add rsp, 40",
    "ret",
    ".seh_endproc",
    ".globl \"?LogWarning@@YAXPEBDZZ\"",
    ".p2align 4",
    "\"?LogWarning@@YAXPEBDZZ\":",
    ".seh_proc \"?LogWarning@@YAXPEBDZZ\"",
    "mov qword ptr [rsp + 8], rcx",
    "mov qword ptr [rsp + 16], rdx",
    "mov qword ptr [rsp + 24], r8",
    "mov qword ptr [rsp + 32], r9",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "lea rdx, [rsp + 56]",
    "call \"?LogWarningV@@YAXPEBDPEAD@Z\"",
    "add rsp, 40",
    "ret",
    ".seh_endproc",
    ".globl \"?LogInfo@@YAXPEBDZZ\"",
    ".p2align 4",
    "\"?LogInfo@@YAXPEBDZZ\":",
    ".seh_proc \"?LogInfo@@YAXPEBDZZ\"",
    "mov qword ptr [rsp + 8], rcx",
    "mov qword ptr [rsp + 16], rdx",
    "mov qword ptr [rsp + 24], r8",
    "mov qword ptr [rsp + 32], r9",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "lea rdx, [rsp + 56]",
    "call \"?LogInfoV@@YAXPEBDPEAD@Z\"",
    "add rsp, 40",
    "ret",
    ".seh_endproc",
    ".globl \"?KdDbgPrint@@YAXPEBD0ZZ\"",
    ".p2align 4",
    "\"?KdDbgPrint@@YAXPEBD0ZZ\":",
    ".seh_proc \"?KdDbgPrint@@YAXPEBD0ZZ\"",
    "mov qword ptr [rsp + 8], rcx",
    "mov qword ptr [rsp + 16], rdx",
    "mov qword ptr [rsp + 24], r8",
    "mov qword ptr [rsp + 32], r9",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "lea r8, [rsp + 64]",
    "call \"?KdDbgPrintV@@YAXPEBD0PEAD@Z\"",
    "add rsp, 40",
    "ret",
    ".seh_endproc",
);

extern "C" {
    #[link_name = "?LogError@@YAXPEBDZZ"]
    pub fn LogError(fmt: *const u8, ...);
    #[link_name = "?LogWarning@@YAXPEBDZZ"]
    pub fn LogWarning(fmt: *const u8, ...);
    #[link_name = "?LogInfo@@YAXPEBDZZ"]
    pub fn LogInfo(fmt: *const u8, ...);
    #[link_name = "?KdDbgPrint@@YAXPEBD0ZZ"]
    pub fn KdDbgPrint(prefix: *const u8, fmt: *const u8, ...);
}

#[allow(dead_code)]
fn wsa_init() -> HRESULT {
    *WSA_INIT.get_or_init(|| unsafe {
        let mut data: WSADATA = core::mem::zeroed();
        hr_from_win32(WSAStartup(0x202, &mut data) as u32)
    })
}

pub fn transport_initialize() -> HRESULT {
    match INIT_HOOK.get() {
        Some(f) => f(),
        None => unsafe { crate::xbtp2_sys::XbtpInitialize() },
    }
}

#[no_mangle]
pub unsafe extern "C" fn XbtpConfigureTransportLogging(level: u32, path: *const u16) -> HRESULT {
    let hr = transport_initialize();
    if hr < 0 {
        return hr;
    }
    if entry(XTF_LOG_SET_MAIN_LEVEL).is_null() {
        return E_NOTIMPL;
    }
    let p = entry(XTF_LOG_SET_MAIN_LEVEL_AND_FILE);
    if p.is_null() {
        return E_NOTIMPL;
    }
    let f: XtfLogSetMainLevelAndFileFn = core::mem::transmute(p);
    f(level, path)
}

#[no_mangle]
pub unsafe extern "C" fn XbtpConfigureMinimumTransportLogging(level: u32) -> HRESULT {
    let p = entry(XTF_LOG_SET_MAIN_LEVEL);
    if p.is_null() {
        return E_NOTIMPL;
    }
    let f: XtfLogSetMainLevelFn = core::mem::transmute(p);
    f(level)
}

pub unsafe fn log_args(level: u16, fmt: &[u8], args: &[usize]) {
    debug_assert_eq!(fmt.last(), Some(&0));
    let threshold = match level {
        2 => 1,
        3 => 3,
        _ => 4,
    };
    if gate() >= threshold {
        InternalLog(level, fmt.as_ptr(), args.as_ptr() as *mut u8);
    }
}

pub unsafe fn log_error_args(fmt: &[u8], args: &[usize]) {
    log_args(2, fmt, args)
}

pub unsafe fn log_warning_args(fmt: &[u8], args: &[usize]) {
    log_args(3, fmt, args)
}

pub unsafe fn log_info_args(fmt: &[u8], args: &[usize]) {
    log_args(4, fmt, args)
}

fn log_str(level: u16, s: &str) {
    let mut text: Vec<u8> = s.bytes().filter(|b| *b != 0).collect();
    text.push(0);
    unsafe { log_args(level, b"%s\0", &[text.as_ptr() as usize]) }
}

pub fn log_error(s: &str) {
    log_str(2, s)
}

pub fn log_warning(s: &str) {
    log_str(3, s)
}

pub fn log_info(s: &str) {
    log_str(4, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{strlen, wide, S_OK};
    use core::sync::atomic::{AtomicI32, AtomicU32};
    use std::sync::Mutex;

    #[derive(Clone, Debug, PartialEq)]
    struct Rec {
        level: u32,
        ctx: usize,
        flags: u32,
        fmt: String,
        a0: u64,
        a1: u64,
    }

    static RECS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());
    static LEVEL: AtomicI32 = AtomicI32::new(0);
    static SET_LEVEL: AtomicU32 = AtomicU32::new(0);
    static SET_FILE: Mutex<String> = Mutex::new(String::new());
    static KD: Mutex<Vec<(String, String, u64, u64)>> = Mutex::new(Vec::new());

    unsafe fn cstr(p: *const u8) -> String {
        String::from_utf8_lossy(core::slice::from_raw_parts(p, strlen(p))).into_owned()
    }

    unsafe extern "C" fn fake_log(level: u32, ctx: *mut c_void, _file: *const u8, _func: *const u8, _line: i32, fmt: *const u8, args: *mut u8, flags: u32) -> i32 {
        let va = args as *const u64;
        let f = cstr(fmt);
        let n = f.matches('%').count();
        let a0 = if n >= 1 { *va } else { 0 };
        let a1 = if n >= 2 { *va.add(1) } else { 0 };
        RECS.lock().unwrap().push(Rec { level, ctx: ctx as usize, flags, fmt: f, a0, a1 });
        0
    }

    unsafe extern "C" fn fake_level() -> i32 {
        LEVEL.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn fake_set(level: u32) -> HRESULT {
        SET_LEVEL.store(level, Ordering::SeqCst);
        S_OK
    }

    unsafe extern "C" fn fake_set_file(level: u32, path: *const u16) -> HRESULT {
        SET_LEVEL.store(level + 100, Ordering::SeqCst);
        *SET_FILE.lock().unwrap() = crate::sys::from_wide(path);
        0x0000_0001
    }

    unsafe extern "system" fn fake_kd(prefix: *const u8, _c: u32, _l: u32, fmt: *const u8, args: *mut u8) -> u32 {
        let va = args as *const u64;
        KD.lock().unwrap().push((cstr(prefix), cstr(fmt), *va, *va.add(1)));
        0
    }

    fn find(fmt: &str) -> Vec<Rec> {
        RECS.lock().unwrap().iter().filter(|r| r.fmt == fmt).cloned().collect()
    }

    #[test]
    fn logging_exports_follow_console_gates() {
        ensure_loaded();
        let had_xtf = !LOG_TABLE[XTF_LOG_SET_MAIN_LEVEL].load(Ordering::SeqCst).is_null();
        if !had_xtf {
            assert_eq!(unsafe { XbtpConfigureMinimumTransportLogging(4) }, E_NOTIMPL);
            assert_eq!(unsafe { XbtpConfigureTransportLogging(4, core::ptr::null()) }, E_NOTIMPL);
            assert_eq!(main_level(), 0);
            unsafe { LogError(b"no-xtf %d\0".as_ptr(), 1i32) };
            log_error("no-xtf plain");
        }
        let saved: Vec<*mut c_void> = LOG_TABLE.iter().map(|p| p.load(Ordering::SeqCst)).collect();
        LOG_TABLE[XTF_LOG_MESSAGE].store(fake_log as *mut c_void, Ordering::SeqCst);
        LOG_TABLE[XTF_LOG_GET_MAIN_LEVEL].store(fake_level as *mut c_void, Ordering::SeqCst);
        LOG_TABLE[XTF_LOG_SET_MAIN_LEVEL].store(fake_set as *mut c_void, Ordering::SeqCst);
        LOG_TABLE[XTF_LOG_SET_MAIN_LEVEL_AND_FILE].store(fake_set_file as *mut c_void, Ordering::SeqCst);
        KD_PRINT.store(fake_kd as *mut c_void, Ordering::SeqCst);

        let s = b"str\0";
        LEVEL.store(4, Ordering::SeqCst);
        unsafe {
            LogError(b"t-err %d %s\0".as_ptr(), 7i32, s.as_ptr());
            LogWarning(b"t-warn %d %s\0".as_ptr(), 8i32, s.as_ptr());
            LogInfo(b"t-info %d %s\0".as_ptr(), 9i32, s.as_ptr());
        }
        let ctx = LOG_CONTEXT.ptr() as usize;
        assert_eq!(find("t-err %d %s"), vec![Rec { level: 2, ctx, flags: 0x11, fmt: "t-err %d %s".into(), a0: 7, a1: s.as_ptr() as u64 }]);
        assert_eq!(find("t-warn %d %s")[0].level, 3);
        assert_eq!(find("t-warn %d %s")[0].a1, s.as_ptr() as u64);
        assert_eq!(find("t-info %d %s")[0].level, 4);
        assert_eq!(find("t-info %d %s")[0].a0, 9);

        let va = [11usize, 12usize];
        unsafe {
            LogErrorV(b"v-err %d %d\0".as_ptr(), va.as_ptr() as *mut u8);
            LogWarningV(b"v-warn %d %d\0".as_ptr(), va.as_ptr() as *mut u8);
            LogInfoV(b"v-info %d %d\0".as_ptr(), va.as_ptr() as *mut u8);
        }
        assert_eq!((find("v-err %d %d")[0].level, find("v-err %d %d")[0].a1), (2, 12));
        assert_eq!(find("v-warn %d %d")[0].level, 3);
        assert_eq!(find("v-info %d %d")[0].a0, 11);

        LEVEL.store(3, Ordering::SeqCst);
        unsafe {
            LogInfo(b"g3-info\0".as_ptr());
            LogInfoV(b"g3-infov\0".as_ptr(), va.as_ptr() as *mut u8);
            LogWarning(b"g3-warn\0".as_ptr());
        }
        assert!(find("g3-info").is_empty() && find("g3-infov").is_empty());
        assert_eq!(find("g3-warn").len(), 1);

        LEVEL.store(1, Ordering::SeqCst);
        unsafe {
            LogWarning(b"g1-warn\0".as_ptr());
            LogWarningV(b"g1-warnv\0".as_ptr(), va.as_ptr() as *mut u8);
            LogError(b"g1-err\0".as_ptr());
        }
        assert!(find("g1-warn").is_empty() && find("g1-warnv").is_empty());
        assert_eq!(find("g1-err").len(), 1);

        LEVEL.store(0, Ordering::SeqCst);
        unsafe {
            LogError(b"g0-err\0".as_ptr());
            LogErrorV(b"g0-errv\0".as_ptr(), va.as_ptr() as *mut u8);
            InternalLog(9, b"g0-internal\0".as_ptr(), va.as_ptr() as *mut u8);
        }
        assert!(find("g0-err").is_empty() && find("g0-errv").is_empty());
        assert_eq!(find("g0-internal")[0].level, 9);
        assert_eq!(find("g0-internal")[0].flags, 0x11);

        LEVEL.store(4, Ordering::SeqCst);
        log_error("rust-err");
        log_warning("rust-warn");
        log_info("rust-info");
        let all = RECS.lock().unwrap().clone();
        let levels: Vec<u32> = all.iter().filter(|r| r.fmt == "%s").map(|r| r.level).collect();
        assert!(levels.contains(&2) && levels.contains(&3) && levels.contains(&4));

        unsafe {
            KdDbgPrint(b"PFX\0".as_ptr(), b"kd %d %d\0".as_ptr(), 21i32, 22i32);
            KdDbgPrintV(b"PFV\0".as_ptr(), b"kdv\0".as_ptr(), va.as_ptr() as *mut u8);
        }
        let kd = KD.lock().unwrap().clone();
        assert!(kd.contains(&("PFX".into(), "kd %d %d".into(), 21, 22)));
        assert!(kd.contains(&("PFV".into(), "kdv".into(), 11, 12)));

        assert_eq!(unsafe { XbtpConfigureMinimumTransportLogging(5) }, S_OK);
        assert_eq!(SET_LEVEL.load(Ordering::SeqCst), 5);
        let path = wide("D:\\logs\\xbtp.log");
        assert_eq!(unsafe { XbtpConfigureTransportLogging(3, path.as_ptr()) }, 1);
        assert_eq!(SET_LEVEL.load(Ordering::SeqCst), 103);
        assert_eq!(&*SET_FILE.lock().unwrap(), "D:\\logs\\xbtp.log");
        LOG_TABLE[XTF_LOG_SET_MAIN_LEVEL].store(core::ptr::null_mut(), Ordering::SeqCst);
        assert_eq!(unsafe { XbtpConfigureTransportLogging(3, path.as_ptr()) }, E_NOTIMPL);
        assert_eq!(unsafe { XbtpConfigureMinimumTransportLogging(3) }, E_NOTIMPL);

        KD_PRINT.store(core::ptr::null_mut(), Ordering::SeqCst);
        for (i, p) in saved.into_iter().enumerate() {
            LOG_TABLE[i].store(p, Ordering::SeqCst);
        }
        unsafe { KdDbgPrint(b"XBTP: \0".as_ptr(), b"real ntdll %d\n\0".as_ptr(), 1i32) };
        if !had_xtf {
            unsafe { log_message(2, core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), 0, b"silent\0".as_ptr(), core::ptr::null_mut(), 0) };
            unsafe { log_message(2, core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), 0, b"fallback\0".as_ptr(), core::ptr::null_mut(), 2) };
        }
    }

    #[test]
    fn transport_initialize_falls_back_to_winsock() {
        assert_eq!(transport_initialize(), S_OK);
        assert_eq!(transport_initialize(), S_OK);
    }
}
