//! USN Journal de NTFS: el registro de cambios del volumen.
//!
//! NTFS anota cada creación, borrado, renombrado o modificación con un número
//! de secuencia (USN). Guardando el último USN visto, al volver a abrir Rayo se
//! leen solo los cambios ocurridos desde entonces, sin reindexar. Mientras está
//! abierto, la lectura bloqueante entrega los cambios al instante sin sondear.
//!
//! Leer el journal requiere administrador: lo hace el servicio de Rayo (o la
//! propia app si se ejecuta elevada). Aplicar los cambios al índice no.

use super::store::*;
use std::collections::HashMap;
use windows::Win32::Foundation::{CloseHandle, ERROR_JOURNAL_DELETE_IN_PROGRESS, ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::*;
use windows::core::PCWSTR;

pub const REASON_DATA: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004 | 0x0000_0010 | 0x0000_0020 | 0x0000_0040;
pub const REASON_CREATE: u32 = 0x0000_0100;
pub const REASON_DELETE: u32 = 0x0000_0200;
pub const REASON_RENAME_OLD: u32 = 0x0000_1000;
pub const REASON_RENAME_NEW: u32 = 0x0000_2000;
pub const REASON_BASIC_INFO: u32 = 0x0000_8000;
pub const REASON_HARD_LINK: u32 = 0x0001_0000;

const REC_MASK: u64 = 0xFFFF_FFFF_FFFF;
const ROOT_RECORD: u32 = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Journal {
    pub id: u64,
    pub first: i64,
    pub next: i64,
}

/// Un cambio del journal, ya con tamaño y fecha del archivo (los agrega quien
/// lee el journal, porque el registro no los trae).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub frn: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attrs: u32,
    pub name: String,
    pub size: u64,
    pub mtime: u32,
}

#[derive(Debug)]
pub enum ReadError {
    /// El journal se reinició o ya no tiene los cambios pedidos: hay que reindexar.
    Reset,
    Other(String),
}

/// Handle de volumen (`\\.\C:`) para el journal y para abrir archivos por id.
pub struct Volume(pub HANDLE);

unsafe impl Send for Volume {}
unsafe impl Sync for Volume {}

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Volume {
    pub fn open(letter: char) -> Result<Self, String> {
        let path: Vec<u16> = format!("\\\\.\\{}:", letter.to_ascii_uppercase()).encode_utf16().chain([0]).collect();
        let h = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                windows::Win32::Foundation::GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .map_err(|e| format!("no se pudo abrir {letter}: ({}); requiere administrador", e.message()))?;
        Ok(Volume(h))
    }

    pub fn journal(&self) -> Result<Journal, String> {
        let mut d = USN_JOURNAL_DATA_V0::default();
        let mut got = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(&mut d as *mut _ as *mut _),
                std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
                Some(&mut got),
                None,
            )
        }
        .map_err(|e| format!("journal no disponible: {}", e.message()))?;
        Ok(Journal { id: d.UsnJournalID, first: d.FirstUsn, next: d.NextUsn })
    }

    /// Lee cambios a partir de `start`. Con `wait_secs > 0` espera hasta que haya
    /// alguno (o se cumpla el tiempo). Devuelve los cambios y el siguiente USN.
    pub fn read(&self, journal_id: u64, start: i64, wait_secs: u32) -> Result<(Vec<Change>, i64), ReadError> {
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: start,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 0,
            Timeout: wait_secs as u64,
            BytesToWaitFor: u64::from(wait_secs > 0),
            UsnJournalID: journal_id,
        };
        let mut buf = vec![0u64; 256 * 1024 / 8];
        let mut got = 0u32;
        let r = unsafe {
            DeviceIoControl(
                self.0,
                FSCTL_READ_USN_JOURNAL,
                Some(&input as *const _ as *const _),
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                Some(buf.as_mut_ptr() as *mut _),
                (buf.len() * 8) as u32,
                Some(&mut got),
                None,
            )
        };
        if let Err(e) = r {
            let code = e.code();
            if code == ERROR_JOURNAL_ENTRY_DELETED.to_hresult() || code == ERROR_JOURNAL_NOT_ACTIVE.to_hresult() || code == ERROR_JOURNAL_DELETE_IN_PROGRESS.to_hresult() {
                return Err(ReadError::Reset);
            }
            // Un id de journal que ya no existe también obliga a reindexar.
            if code == windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult() {
                return Err(ReadError::Reset);
            }
            return Err(ReadError::Other(e.message()));
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
        Ok(parse_records(bytes))
    }

    /// Tamaño, fecha y atributos de un archivo abierto por su número de registro.
    pub fn stat(&self, frn: u64) -> Option<(u64, u32, u32)> {
        let desc = FILE_ID_DESCRIPTOR {
            dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
            Type: FileIdType,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 },
        };
        unsafe {
            let h = OpenFileById(self.0, &desc, FILE_READ_ATTRIBUTES.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, FILE_FLAG_BACKUP_SEMANTICS).ok()?;
            let mut basic = FILE_BASIC_INFO::default();
            let mut std_info = FILE_STANDARD_INFO::default();
            let ok = GetFileInformationByHandleEx(h, FileBasicInfo, &mut basic as *mut _ as *mut _, std::mem::size_of::<FILE_BASIC_INFO>() as u32).is_ok()
                && GetFileInformationByHandleEx(h, FileStandardInfo, &mut std_info as *mut _ as *mut _, std::mem::size_of::<FILE_STANDARD_INFO>() as u32).is_ok();
            let _ = CloseHandle(h);
            ok.then(|| (if std_info.Directory { 0 } else { std_info.EndOfFile as u64 }, filetime_to_unix(basic.LastWriteTime as u64), basic.FileAttributes))
        }
    }

    /// Lee y completa (tamaño y fecha) los cambios desde `start`.
    pub fn changes(&self, journal_id: u64, start: i64, wait_secs: u32) -> Result<(Vec<Change>, i64), ReadError> {
        let (mut list, next) = self.read(journal_id, start, wait_secs)?;
        for c in &mut list {
            if c.reason & REASON_DELETE == 0 && c.reason & REASON_RENAME_OLD == 0 {
                if let Some((size, mtime, attrs)) = self.stat(c.frn) {
                    c.size = size;
                    c.mtime = mtime;
                    c.attrs = attrs;
                }
            }
        }
        Ok((list, next))
    }
}

fn u16at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Interpreta la salida de FSCTL_READ_USN_JOURNAL: 8 bytes con el siguiente USN
/// y luego registros USN_RECORD_V2 o V3.
pub fn parse_records(b: &[u8]) -> (Vec<Change>, i64) {
    if b.len() < 8 {
        return (Vec::new(), 0);
    }
    let next = u64at(b, 0) as i64;
    let mut out = Vec::new();
    let mut p = 8usize;
    while p + 60 <= b.len() {
        let len = u32at(b, p) as usize;
        if len < 60 || p + len > b.len() {
            break;
        }
        let r = &b[p..p + len];
        let major = u16at(r, 4);
        let (frn, parent, usn, reason, attrs, name_len, name_off) = match major {
            2 => (u64at(r, 8), u64at(r, 16), u64at(r, 24) as i64, u32at(r, 40), u32at(r, 52), u16at(r, 56) as usize, u16at(r, 58) as usize),
            // V3: identificadores de 128 bits; en NTFS caben en los 64 bajos.
            3 if len >= 76 => (u64at(r, 8), u64at(r, 24), u64at(r, 40) as i64, u32at(r, 56), u32at(r, 68), u16at(r, 72) as usize, u16at(r, 74) as usize),
            _ => {
                p += len;
                continue;
            }
        };
        if name_off + name_len <= len {
            let units: Vec<u16> = (0..name_len / 2).map(|k| u16at(r, name_off + k * 2)).collect();
            out.push(Change { frn, parent, usn, reason, attrs, name: String::from_utf16_lossy(&units), size: 0, mtime: 0 });
        }
        p += len;
    }
    (out, next)
}

// ---------------------------------------------------------------- aplicar cambios

/// Estado para aplicar cambios: carpetas por número de registro y renombres a medias.
#[derive(Default)]
pub struct Applier {
    pending_old: HashMap<u64, u32>,
}

impl Applier {
    fn dir_entry(v: &VolumeIndex, frn: u64) -> Option<u32> {
        let rec = (frn & REC_MASK) as u32;
        if rec == ROOT_RECORD {
            return Some(0);
        }
        v.dir_map.get(&rec).copied().filter(|&e| !v.is_deleted(e))
    }

    /// Aplica un cambio. Devuelve la carpeta afectada (para refrescar la vista).
    pub fn apply(&mut self, v: &mut VolumeIndex, c: &Change) -> Option<u32> {
        let rec = (c.frn & REC_MASK) as u32;
        let parent = Self::dir_entry(v, c.parent);
        let is_dir = c.attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;

        if c.reason & REASON_RENAME_OLD != 0 {
            // Primera mitad de un renombrado/movimiento: recordar qué entrada era.
            let p = parent?;
            let e = v.find_child(p, &c.name)?;
            self.pending_old.insert(c.frn, e);
            return Some(p);
        }
        if c.reason & REASON_DELETE != 0 {
            let p = parent?;
            let e = v.find_child(p, &c.name)?;
            if v.is_dir(e) {
                v.dir_map.remove(&rec);
            }
            v.remove_subtree(e);
            return Some(p);
        }
        let p = parent?;
        if c.reason & REASON_RENAME_NEW != 0 {
            if let Some(old) = self.pending_old.remove(&c.frn) {
                if !v.is_deleted(old) {
                    let new = v.rename_entry(old, p, &c.name, c.mtime);
                    if v.is_dir(new) {
                        v.dir_map.insert(rec, new);
                    }
                    return Some(p);
                }
            }
        }
        match v.find_child(p, &c.name) {
            Some(e) => {
                if c.mtime != 0 {
                    v.update_file(e, c.size, c.mtime);
                }
            }
            None => {
                let flags = super::walk::attrs_to_flags(c.attrs) | if is_dir { F_DIR } else { 0 };
                let e = v.add_entry(p, &c.name, flags, if is_dir { 0 } else { c.size }, c.mtime);
                if is_dir {
                    v.dir_map.insert(rec, e);
                }
            }
        }
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_v2(frn: u64, parent: u64, usn: i64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let len = (60 + units.len() * 2).next_multiple_of(8);
        let mut r = vec![0u8; len];
        r[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        r[4..6].copy_from_slice(&2u16.to_le_bytes());
        r[8..16].copy_from_slice(&frn.to_le_bytes());
        r[16..24].copy_from_slice(&parent.to_le_bytes());
        r[24..32].copy_from_slice(&usn.to_le_bytes());
        r[40..44].copy_from_slice(&reason.to_le_bytes());
        r[52..56].copy_from_slice(&attrs.to_le_bytes());
        r[56..58].copy_from_slice(&((units.len() * 2) as u16).to_le_bytes());
        r[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (k, u) in units.iter().enumerate() {
            r[60 + k * 2..62 + k * 2].copy_from_slice(&u.to_le_bytes());
        }
        r
    }

    #[test]
    fn parses_records() {
        let mut b = 777i64.to_le_bytes().to_vec();
        b.extend(record_v2(100, 5, 10, REASON_CREATE, 0x20, "nuevo.txt"));
        b.extend(record_v2(101, 100, 11, REASON_DELETE, 0x10, "Canción"));
        let (list, next) = parse_records(&b);
        assert_eq!(next, 777);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "nuevo.txt");
        assert_eq!(list[1].name, "Canción");
        assert_eq!(list[1].parent, 100);
    }

    fn ch(frn: u64, parent: u64, reason: u32, dir: bool, name: &str, size: u64) -> Change {
        Change { frn, parent, reason, attrs: if dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { 0x20 }, name: name.into(), size, mtime: 100, ..Default::default() }
    }

    #[test]
    fn applies_create_rename_move_delete() {
        let mut v = VolumeIndex::new('c', Source::Mft);
        v.compute_dir_sizes();
        let mut a = Applier::default();
        // Crear C:\Fotos (registro 40) y C:\Fotos\a.jpg (registro 41).
        a.apply(&mut v, &ch(40, 5, REASON_CREATE, true, "Fotos", 0));
        a.apply(&mut v, &ch(41, 40, REASON_CREATE, false, "a.jpg", 1000));
        let file = v.find_path("C:\\Fotos\\a.jpg").unwrap();
        assert_eq!(v.size[0], 1000);
        // Modificar el archivo.
        a.apply(&mut v, &ch(41, 40, REASON_DATA, false, "a.jpg", 1500));
        assert_eq!(v.size[file as usize], 1500);
        assert_eq!(v.size[0], 1500);
        // Crear C:\Archivo (registro 50) y mover Fotos dentro con otro nombre.
        a.apply(&mut v, &ch(50, 5, REASON_CREATE, true, "Archivo", 0));
        a.apply(&mut v, &ch(40, 5, REASON_RENAME_OLD, true, "Fotos", 0));
        a.apply(&mut v, &ch(40, 50, REASON_RENAME_NEW, true, "Fotos 2024", 0));
        assert_eq!(v.find_path("C:\\Fotos"), None);
        let moved = v.find_path("C:\\Archivo\\Fotos 2024\\a.jpg").unwrap();
        assert_eq!(v.full_path(moved), "C:\\Archivo\\Fotos 2024\\a.jpg");
        let archivo = v.find_path("C:\\Archivo").unwrap();
        assert_eq!(v.size[archivo as usize], 1500);
        assert_eq!(v.depth(moved), 3);
        // Un archivo nuevo dentro de la carpeta movida llega por su registro (40).
        a.apply(&mut v, &ch(42, 40, REASON_CREATE, false, "b.jpg", 10));
        assert!(v.find_path("C:\\Archivo\\Fotos 2024\\b.jpg").is_some());
        // Borrar la carpeta movida con todo su contenido.
        a.apply(&mut v, &ch(40, 50, REASON_DELETE, true, "Fotos 2024", 0));
        assert_eq!(v.find_path("C:\\Archivo\\Fotos 2024"), None);
        assert_eq!(v.size[0], 0);
    }
}
