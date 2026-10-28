//! Qué clase de disco hay detrás de una ruta (NVMe, SSD, disco mecánico, USB o red) y en qué disco
//! físico está. Las copias lo usan para elegir la estrategia más rápida en cada caso:
//!
//! - entre discos sin cabezal (NVMe, SSD) se copian varios archivos a la vez;
//! - si hay un disco mecánico o un USB de por medio, de a uno (en paralelo el cabezal salta y es más lento);
//! - dos copias que comparten un disco físico se hacen una después de la otra, y las que no, a la vez.
//!
//! Nada de esto pide permisos: el volumen se abre sin acceso de lectura, solo para preguntarle.

use super::{long_path, short_path};
use parking_lot::Mutex;
use std::collections::HashMap;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::*;
use windows::core::PCWSTR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Nvme,
    Ssd,
    Hdd,
    Usb,
    Network,
    Unknown,
}

impl Kind {
    /// Aguanta varias lecturas y escrituras a la vez sin perder velocidad (no hay cabezal que mover;
    /// en red, varias a la vez esconden la demora de cada pedido).
    pub fn parallel_friendly(self) -> bool {
        matches!(self, Kind::Nvme | Kind::Ssd | Kind::Network)
    }
}

#[derive(Clone, Debug)]
pub struct Volume {
    pub kind: Kind,
    /// Discos físicos que ocupa ("disco:0") o, en red, el servidor ("red:nas").
    pub disks: Vec<String>,
}

static CACHE: Mutex<Option<HashMap<String, Volume>>> = Mutex::new(None);

/// El volumen de una ruta (se pregunta a Windows una sola vez por volumen).
pub fn volume(path: &str) -> Volume {
    let root = volume_root(path);
    if let Some(v) = CACHE.lock().as_ref().and_then(|c| c.get(&root)) {
        return v.clone();
    }
    let v = query(&root);
    CACHE.lock().get_or_insert_with(HashMap::new).insert(root, v.clone());
    v
}

/// Todos los discos físicos que toca una operación (fuentes y destino), sin repetir.
pub fn disks_of<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        for d in volume(p).disks {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// "C:\carpeta\x" → "c:\"; "\\nas\fotos\x" → "\\nas\fotos"; puntos de montaje → su carpeta.
fn volume_root(path: &str) -> String {
    let p = short_path(path);
    if let Some(rest) = p.strip_prefix(r"\\") {
        let mut parts = rest.splitn(3, '\\');
        return format!(r"\\{}\{}", parts.next().unwrap_or(""), parts.next().unwrap_or("")).to_lowercase();
    }
    let w: Vec<u16> = long_path(&p).encode_utf16().chain([0]).collect();
    let mut buf = [0u16; 1024];
    let root = unsafe { GetVolumePathNameW(PCWSTR(w.as_ptr()), &mut buf) }
        .ok()
        .map(|_| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| p.get(..3).unwrap_or("").to_string());
    short_path(&root).to_lowercase()
}

fn query(root: &str) -> Volume {
    if let Some(rest) = root.strip_prefix(r"\\") {
        let server = rest.split('\\').next().unwrap_or(rest);
        return Volume { kind: Kind::Network, disks: vec![format!("red:{server}")] };
    }
    let wroot: Vec<u16> = root.encode_utf16().chain([0]).collect();
    if unsafe { GetDriveTypeW(PCWSTR(wroot.as_ptr())) } == 4 {
        // DRIVE_REMOTE: una unidad de red con letra.
        return Volume { kind: Kind::Network, disks: vec![format!("red:{root}")] };
    }
    // El dispositivo del volumen: "\\.\C:" o, para puntos de montaje, "\\?\Volume{…}".
    let device = if root.len() <= 3 && root.as_bytes().get(1) == Some(&b':') {
        format!(r"\\.\{}", &root[..2])
    } else {
        let mut name = [0u16; 64];
        let ok = unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(wroot.as_ptr()), &mut name) }.is_ok();
        if !ok {
            return Volume { kind: Kind::Unknown, disks: vec![format!("vol:{root}")] };
        }
        String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)]).trim_end_matches('\\').to_string()
    };
    let wdev: Vec<u16> = device.encode_utf16().chain([0]).collect();
    let Ok(h) = (unsafe { CreateFileW(PCWSTR(wdev.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }) else {
        return Volume { kind: Kind::Unknown, disks: vec![format!("vol:{root}")] };
    };
    let seek_penalty = seek_penalty(h);
    let bus = bus_type(h);
    let mut disks = disk_numbers(h).into_iter().map(|n| format!("disco:{n}")).collect::<Vec<_>>();
    unsafe {
        let _ = CloseHandle(h);
    }
    if disks.is_empty() {
        disks.push(format!("vol:{root}"));
    }
    let kind = match (bus, seek_penalty) {
        (Some(b), _) if b == BusTypeUsb => Kind::Usb,
        (_, Some(true)) => Kind::Hdd,
        (Some(b), _) if b == BusTypeNvme => Kind::Nvme,
        (_, Some(false)) => Kind::Ssd,
        _ => Kind::Unknown,
    };
    Volume { kind, disks }
}

fn ioctl<T>(h: HANDLE, code: u32, input: Option<&[u8]>, out: &mut T) -> bool {
    let mut got = 0u32;
    unsafe {
        DeviceIoControl(
            h,
            code,
            input.map(|i| i.as_ptr() as *const _),
            input.map_or(0, |i| i.len() as u32),
            Some(out as *mut T as *mut _),
            std::mem::size_of::<T>() as u32,
            Some(&mut got),
            None,
        )
        .is_ok()
    }
}

fn property_query(id: STORAGE_PROPERTY_ID) -> Vec<u8> {
    let q = STORAGE_PROPERTY_QUERY { PropertyId: id, QueryType: PropertyStandardQuery, ..Default::default() };
    let bytes = unsafe { std::slice::from_raw_parts(&q as *const _ as *const u8, std::mem::size_of::<STORAGE_PROPERTY_QUERY>()) };
    bytes.to_vec()
}

/// Si el disco tiene cabezal (tarda en saltar de un lugar a otro).
fn seek_penalty(h: HANDLE) -> Option<bool> {
    let mut d = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
    ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&property_query(StorageDeviceSeekPenaltyProperty)), &mut d).then(|| d.IncursSeekPenalty)
}

/// Cómo está conectado (NVMe, SATA, USB…).
fn bus_type(h: HANDLE) -> Option<STORAGE_BUS_TYPE> {
    // El descriptor trae textos detrás de la estructura: se lee en un búfer más grande.
    #[repr(C, align(8))]
    struct Buf([u8; 1024]);
    let mut buf = Buf([0; 1024]);
    if !ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&property_query(StorageDeviceProperty)), &mut buf) {
        return None;
    }
    let d = unsafe { &*(buf.0.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    Some(d.BusType)
}

/// Los discos físicos del volumen (uno normalmente; varios en un volumen distribuido).
fn disk_numbers(h: HANDLE) -> Vec<u32> {
    #[repr(C)]
    struct Extents {
        count: u32,
        _pad: u32,
        list: [DISK_EXTENT; 8],
    }
    let mut e = Extents { count: 0, _pad: 0, list: [DISK_EXTENT::default(); 8] };
    if !ioctl(h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None, &mut e) {
        return Vec::new();
    }
    let mut v: Vec<u32> = e.list[..(e.count as usize).min(8)].iter().map(|x| x.DiskNumber).collect();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_drive_is_detected() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let v = volume(&windir);
        println!("{windir}: {:?} en {:?}", v.kind, v.disks);
        assert!(!v.disks.is_empty());
        // La misma unidad, por otra ruta, es el mismo disco.
        let again = volume(&format!("{}\\System32", windir));
        assert_eq!(v.disks, again.disks);
    }

    #[test]
    fn network_paths_are_network() {
        let v = volume(r"\\servidor\compartida\carpeta");
        assert_eq!(v.kind, Kind::Network);
        assert_eq!(v.disks, vec!["red:servidor".to_string()]);
    }
}
