use core::ffi::c_void;
use core::ptr::null_mut;
use repository::PackageXref;

pub mod repository;

pub type Hkey = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn PackageFamilyNameFromFullName(full: *const u16, len: *mut u32, family: *mut u16) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenCurrentUser(sam: u32, out: *mut Hkey) -> i32;
    fn RegOpenKeyExW(key: Hkey, sub: *const u16, opts: u32, sam: u32, out: *mut Hkey) -> i32;
    fn RegQueryValueExW(key: Hkey, name: *const u16, res: *mut u32, ty: *mut u32, data: *mut u8, len: *mut u32) -> i32;
    fn RegCloseKey(key: Hkey) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

pub const ERROR_FILE_NOT_FOUND: i32 = 2;
pub const ERROR_INVALID_PARAMETER: i32 = 0x57;
pub const ERROR_NOT_FOUND: i32 = 0x490;
pub const ERROR_REGISTRY_CORRUPT_TYPE: i32 = 0x3d55;
pub const KEY_READ: u32 = 0x20019;
pub const REG_QWORD: u32 = 0xb;
pub const PATH_CAP: usize = 0x128;
pub const FAMILIES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Families";

unsafe fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub unsafe fn install_time_under(root: Hkey, prefix: &str, full: *const u16, out: *mut FileTime) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut family = [0u16; 65];
    let mut flen = family.len() as u32;
    let r = PackageFamilyNameFromFullName(full, &mut flen, family.as_mut_ptr());
    if r != 0 {
        return r;
    }
    let mut path: Vec<u16> = prefix.encode_utf16().collect();
    path.push(b'\\' as u16);
    path.extend_from_slice(&family[..wlen(family.as_ptr())]);
    path.push(b'\\' as u16);
    path.extend_from_slice(core::slice::from_raw_parts(full, wlen(full)));
    if path.len() >= PATH_CAP {
        return ERROR_INVALID_PARAMETER;
    }
    path.push(0);
    let mut key: Hkey = null_mut();
    let r = RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut key);
    if r != 0 {
        return if r == ERROR_FILE_NOT_FOUND { ERROR_NOT_FOUND } else { r };
    }
    let name: Vec<u16> = "InstallTime".encode_utf16().chain(Some(0)).collect();
    let mut ty = 0u32;
    let mut data = 0u64;
    let mut len = 8u32;
    let r = RegQueryValueExW(key, name.as_ptr(), null_mut(), &mut ty, &mut data as *mut u64 as *mut u8, &mut len);
    RegCloseKey(key);
    if r != 0 {
        return r;
    }
    if ty != REG_QWORD {
        return ERROR_REGISTRY_CORRUPT_TYPE;
    }
    (*out).low = data as u32;
    (*out).high = (data >> 32) as u32;
    0
}

unsafe fn name_copy(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    let n = wlen(p);
    if n == 0 || n >= PATH_CAP {
        return None;
    }
    Some(core::slice::from_raw_parts(p, n).to_vec())
}

unsafe fn repository_install_time(full: *const u16) -> Option<u64> {
    let mut family = [0u16; 65];
    let mut flen = family.len() as u32;
    if PackageFamilyNameFromFullName(full, &mut flen, family.as_mut_ptr()) != 0 {
        return None;
    }
    let family = &family[..wlen(family.as_ptr())];
    let full = core::slice::from_raw_parts(full, wlen(full));
    repository::get(family, full).map(|x| x.install_time)
}

/// Console KernelBase deployment internal: adds one package-family xref under
/// `AppModel\Repository\Families\<family>\<fullname>`. The emulator keeps the
/// same `InstallTime` qword and extra dword in its in-process repository, never
/// in the signed-in user's real registry hive.
#[no_mangle]
pub unsafe extern "system" fn AddPackageToFamilyXref(family: *const u16, full: *const u16, xref: PackageXref) -> i32 {
    let Some(family) = name_copy(family) else {
        return ERROR_INVALID_PARAMETER;
    };
    let Some(full_name) = name_copy(full) else {
        return ERROR_INVALID_PARAMETER;
    };
    let mut derived = [0u16; 65];
    let mut dlen = derived.len() as u32;
    let r = PackageFamilyNameFromFullName(full, &mut dlen, derived.as_mut_ptr());
    if r != 0 {
        return r;
    }
    repository::add(&family, &full_name, xref);
    0
}

/// Console KernelBase deployment internal paired with
/// [`AddPackageToFamilyXref`]: removes the same `Families\<family>\<fullname>`
/// record from the in-process repository.
#[no_mangle]
pub unsafe extern "system" fn RemovePackageFromFamilyXref(family: *const u16, full: *const u16) -> i32 {
    let Some(family) = name_copy(family) else {
        return ERROR_INVALID_PARAMETER;
    };
    let Some(full) = name_copy(full) else {
        return ERROR_INVALID_PARAMETER;
    };
    if repository::remove(&family, &full) {
        0
    } else {
        ERROR_FILE_NOT_FOUND
    }
}

#[no_mangle]
pub unsafe extern "system" fn GetPackageInstallTime(full: *const u16, out: *mut FileTime) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if let Some(install_time) = repository_install_time(full) {
        (*out).low = install_time as u32;
        (*out).high = (install_time >> 32) as u32;
        return 0;
    }
    let mut user: Hkey = null_mut();
    let r = RegOpenCurrentUser(KEY_READ, &mut user);
    if r != 0 {
        return r;
    }
    let r = install_time_under(user, FAMILIES, full, out);
    RegCloseKey(user);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr::null;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(key: Hkey, sub: *const u16, r: u32, class: *const u16, opts: u32, sam: u32, sa: *const c_void, out: *mut Hkey, disp: *mut u32) -> i32;
        fn RegSetValueExW(key: Hkey, name: *const u16, r: u32, ty: u32, data: *const u8, len: u32) -> i32;
        fn RegDeleteTreeW(key: Hkey, sub: *const u16) -> i32;
    }

    const HKCU: Hkey = 0x8000_0001usize as Hkey;
    const FULL: &str = "TomBox.Test_1.0.0.0_x64__8wekyb3d8bbwe";
    const FAMILY: &str = "TomBox.Test_8wekyb3d8bbwe";

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn seed(prefix: &str, ty: u32, data: &[u8]) {
        unsafe {
            let mut k: Hkey = null_mut();
            let sub = w(&format!("{prefix}\\{FAMILY}\\{FULL}"));
            assert_eq!(RegCreateKeyExW(HKCU, sub.as_ptr(), 0, null(), 0, 0xf003f, null(), &mut k, null_mut()), 0);
            assert_eq!(RegSetValueExW(k, w("InstallTime").as_ptr(), 0, ty, data.as_ptr(), data.len() as u32), 0);
            RegCloseKey(k);
        }
    }

    fn wipe(prefix: &str) {
        unsafe { RegDeleteTreeW(HKCU, w(prefix).as_ptr()) };
    }

    #[test]
    fn reads_qword_from_families_key() {
        let p = format!("Software\\TomBoxTest-{}-a", std::process::id());
        seed(&p, REG_QWORD, &0x01d9_1234_5678_9abcu64.to_le_bytes());
        let mut ft = FileTime::default();
        let r = unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) };
        wipe(&p);
        assert_eq!(r, 0);
        assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
    }

    #[test]
    fn wrong_type_is_corrupt_type() {
        let p = format!("Software\\TomBoxTest-{}-b", std::process::id());
        seed(&p, 4, &7u32.to_le_bytes());
        let mut ft = FileTime { low: 9, high: 9 };
        let r = unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) };
        wipe(&p);
        assert_eq!(r, ERROR_REGISTRY_CORRUPT_TYPE);
        assert_eq!(ft, FileTime { low: 9, high: 9 });
    }

    #[test]
    fn absent_key_is_not_found() {
        let p = format!("Software\\TomBoxTest-{}-c", std::process::id());
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) }, ERROR_NOT_FOUND);
    }

    #[test]
    fn overlong_path_is_invalid_parameter() {
        let p = "x".repeat(PATH_CAP);
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) }, ERROR_INVALID_PARAMETER);
    }

    #[test]
    fn package_xref_round_trip_flows_into_install_time() {
        let suffix = std::process::id();
        let family = format!("TomBox.Repo-{suffix}_8wekyb3d8bbwe");
        let full = format!("TomBox.Repo-{suffix}_1.0.0.0_x64__8wekyb3d8bbwe");
        let family_w = w(&family);
        let full_w = w(&full);
        let x = PackageXref { install_time: 0x01d9_1234_5678_9abc, flags: 7, reserved: 9 };
        unsafe {
            assert_eq!(AddPackageToFamilyXref(family_w.as_ptr(), full_w.as_ptr(), x), 0);
            let mut ft = FileTime::default();
            assert_eq!(GetPackageInstallTime(full_w.as_ptr(), &mut ft), 0);
            assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
            assert_eq!(RemovePackageFromFamilyXref(family_w.as_ptr(), full_w.as_ptr()), 0);
            assert_eq!(GetPackageInstallTime(full_w.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            assert_eq!(RemovePackageFromFamilyXref(family_w.as_ptr(), full_w.as_ptr()), ERROR_FILE_NOT_FOUND);
        }
    }

    #[test]
    fn package_xref_rejects_null_and_empty_names() {
        let family = w("TomBox.Repo_8wekyb3d8bbwe");
        let full = w("TomBox.Repo_1.0.0.0_x64__8wekyb3d8bbwe");
        let empty = w("");
        let x = PackageXref::default();
        unsafe {
            assert_eq!(AddPackageToFamilyXref(core::ptr::null(), full.as_ptr(), x), ERROR_INVALID_PARAMETER);
            assert_eq!(AddPackageToFamilyXref(family.as_ptr(), core::ptr::null(), x), ERROR_INVALID_PARAMETER);
            assert_eq!(AddPackageToFamilyXref(empty.as_ptr(), full.as_ptr(), x), ERROR_INVALID_PARAMETER);
            assert_eq!(RemovePackageFromFamilyXref(core::ptr::null(), full.as_ptr()), ERROR_INVALID_PARAMETER);
            assert_eq!(RemovePackageFromFamilyXref(family.as_ptr(), empty.as_ptr()), ERROR_INVALID_PARAMETER);
        }
    }
}
