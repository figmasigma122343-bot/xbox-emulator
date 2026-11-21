//! In-process console package-family repository.
//!
//! The console KernelBase deployment internals write the package xref under
//! `HKEY_USERS\<sid>\...\AppModel\Repository\Families\<family>\<fullname>`.
//! The emulator must not write into the real user hive, so the same records are
//! kept process-locally and `GetPackageInstallTime` reads them before falling
//! back to the host registry.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// The console xref record: the `InstallTime` qword and the extra dword.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackageXref {
    pub install_time: u64,
    pub flags: u32,
    pub reserved: u32,
}

type Key = (Vec<u16>, Vec<u16>);

static STORE: OnceLock<Mutex<BTreeMap<Key, PackageXref>>> = OnceLock::new();

fn store() -> MutexGuard<'static, BTreeMap<Key, PackageXref>> {
    STORE.get_or_init(|| Mutex::new(BTreeMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn add(family: &[u16], full: &[u16], xref: PackageXref) {
    store().insert((family.to_vec(), full.to_vec()), xref);
}

/// Returns `true` when an entry was removed, `false` when it was absent.
pub fn remove(family: &[u16], full: &[u16]) -> bool {
    store().remove(&(family.to_vec(), full.to_vec())).is_some()
}

pub fn get(family: &[u16], full: &[u16]) -> Option<PackageXref> {
    store().get(&(family.to_vec(), full.to_vec())).copied()
}

/// Test helper; the production code only uses [`add`], [`remove`] and [`get`].
#[cfg(test)]
pub fn clear() {
    store().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_get_remove_round_trip() {
        let family: Vec<u16> = "TomBox.Repo_8wekyb3d8bbwe".encode_utf16().collect();
        let full: Vec<u16> = "TomBox.Repo_1.0.0.0_x64__8wekyb3d8bbwe".encode_utf16().collect();
        let x = PackageXref { install_time: 0x01d9_1234_5678_9abc, flags: 7, reserved: 9 };
        add(&family, &full, x);
        assert_eq!(get(&family, &full), Some(x));
        assert!(remove(&family, &full));
        assert_eq!(get(&family, &full), None);
        assert!(!remove(&family, &full));
    }
}
