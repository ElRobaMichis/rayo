//! Benchmark del motor de búsqueda.
//!
//! Uso: rayo-bench [--walk | --mft] [--cache] [letras...]
//!
//! Copias: rayo-bench --copia <origen> <carpeta-de-destino> [veces]
//! compara el motor de copia del Explorador (IFileOperation) con el de Rayo, alternando el orden.

use rayo::index::search::{self, Compiled, Hit};
use rayo::index::{ResultSort, VolumeIndex, cache, mft, ntfs_fixed_volumes, walk};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// (consulta para Rayo, misma consulta en sintaxis de Everything)
const QUERIES: &[(&str, &str)] = &[
    ("a", "a"),
    ("e", "e"),
    ("config", "config"),
    ("readme", "readme"),
    ("node_modules", "node_modules"),
    ("kernel32", "kernel32"),
    (".dll", ".dll"),
    ("factura", "factura"),
    ("canción", "canción"),
    ("foto 2024", "foto 2024"),
    ("ext:pdf", "ext:pdf"),
    ("ext:jpg;png", "ext:jpg;png"),
    ("size:>1gb", "size:>1gb"),
    ("dm:hoy", "dm:today"),
    ("*.mp4", "*.mp4"),
    ("windows\\system32 kernel", "windows\\system32\\ kernel"),
    ("zzqxjv", "zzqxjv"),
];

fn mb(b: usize) -> f64 {
    b as f64 / 1024.0 / 1024.0
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn fmt(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us < 1000.0 { format!("{us:.0} µs") } else { format!("{:.2} ms", us / 1000.0) }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let force_walk = args.iter().any(|a| a == "--walk");
    let force_mft = args.iter().any(|a| a == "--mft");
    let from_cache = args.iter().any(|a| a == "--cache");
    if args.iter().any(|a| a == "--servicio") {
        service_test();
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--copia") {
        let (Some(src), Some(dst)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("uso: rayo-bench --copia <origen> <carpeta-de-destino> [veces]");
            return;
        };
        copy_bench(src, dst, args.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(3));
        return;
    }
    let mut letters: Vec<char> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .filter_map(|a| a.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        letters = ntfs_fixed_volumes();
    }
    println!("Rayo bench · volúmenes: {letters:?} · hilos: {}\n", rayon::current_num_threads());

    let mut vols: Vec<VolumeIndex> = Vec::new();
    for &l in &letters {
        let progress = AtomicUsize::new(0);
        let t = Instant::now();
        let v = if from_cache {
            match cache::load(l) {
                Ok(v) => v,
                Err(e) => {
                    println!("{l}: sin caché ({e})");
                    continue;
                }
            }
        } else if force_walk {
            walk::walk_volume(l, &progress)
        } else {
            match mft::read_mft(l, &progress) {
                Ok(v) => v,
                Err(e) if !force_mft => {
                    println!("{l}: MFT no disponible ({e}); usando recorrido");
                    progress.store(0, Ordering::Relaxed);
                    walk::walk_volume(l, &progress)
                }
                Err(e) => {
                    println!("{l}: {e}");
                    continue;
                }
            }
        };
        let build = t.elapsed();
        println!(
            "{l}: {} entradas · fuente {} · {:.2} s · {:.1} MB en RAM · raíz {:.1} GB · {} nombres con acento",
            v.live_len(),
            v.source.label(),
            build.as_secs_f64(),
            mb(v.memory_bytes()),
            v.size[0] as f64 / 1e9,
            v.accented_count()
        );
        if !from_cache {
            let t = Instant::now();
            cache::save(&v).ok();
            let save = t.elapsed();
            let t = Instant::now();
            let _ = cache::load(l);
            println!("   caché: guardar {} · cargar {}", fmt(save), fmt(t.elapsed()));
        }
        vols.push(v);
    }
    if vols.is_empty() {
        return;
    }
    if args.iter().any(|a| a == "--acentos") {
        for v in &vols {
            let step = (v.acc_ids.len() / 25).max(1);
            for &id in v.acc_ids.iter().step_by(step).take(25) {
                println!("  {}  ←  {}", v.name(id), v.parent_path(id));
            }
        }
        return;
    }

    let ev = rayo::index::everything::Everything::connect();
    match &ev {
        Some(_) => println!("\nEverything detectado: se compara por IPC (top 1000 por consulta, igual que Rayo)."),
        None => println!("\nEverything no está abierto: se omite su columna."),
    }
    println!("\n{:<28} {:>9} {:>11} {:>11} {:>9} {:>11} {:>8}", "consulta", "result.", "ingenuo", "rayo", "ev.res.", "everything", "rayo ×");
    let mut total_rayo = Duration::ZERO;
    let mut total_ev = Duration::ZERO;
    for &(qs, evq) in QUERIES {
        let q = search::parse_query(qs);
        // Motor de referencia: un hilo, entrada por entrada.
        let mut naive_t = Vec::new();
        let mut count = 0;
        for _ in 0..3 {
            let t = Instant::now();
            let mut hits: Vec<Hit> = Vec::new();
            for (k, v) in vols.iter().enumerate() {
                hits.extend(search::search_naive(v, &q, k as u8));
            }
            count = hits.len();
            search::rank(&mut hits, 1000);
            naive_t.push(t.elapsed());
        }
        // Rayo: barrido SIMD en paralelo.
        let mut rayo_t = Vec::new();
        let mut rcount = 0;
        for _ in 0..15 {
            let t = Instant::now();
            let c = Compiled::new(&q);
            let mut hits: Vec<Hit> = Vec::new();
            rcount = 0;
            for (k, v) in vols.iter().enumerate() {
                let (h, n) = search::search_volume(v, &c, k as u8, 1000);
                hits.extend(h);
                rcount += n;
            }
            search::rank(&mut hits, 1000);
            rayo_t.push(t.elapsed());
        }
        assert_eq!(count, rcount, "los motores no coinciden en '{qs}'");
        let (n, r) = (median(naive_t), median(rayo_t));
        total_rayo += r;
        let (ev_count, ev_time) = match &ev {
            Some(ev) => {
                let mut times = Vec::new();
                let mut count = 0;
                for _ in 0..15 {
                    match ev.query(evq, 1000) {
                        Some(res) => {
                            count = res.total;
                            times.push(res.elapsed);
                        }
                        None => break,
                    }
                }
                if times.is_empty() { (String::from("-"), None) } else { (count.to_string(), Some(median(times))) }
            }
            None => (String::from("-"), None),
        };
        if let Some(e) = ev_time {
            total_ev += e;
        }
        let vs = ev_time.map_or(String::from("-"), |e| format!("{:.1}×", e.as_secs_f64() / r.as_secs_f64()));
        println!("{:<28} {:>9} {:>11} {:>11} {:>9} {:>11} {:>8}", qs, rcount, fmt(n), fmt(r), ev_count, ev_time.map_or(String::from("-"), fmt), vs);
    }
    println!("\nTiempo medio por consulta · rayo: {}", fmt(total_rayo / QUERIES.len() as u32));
    if ev.is_some() {
        println!("Tiempo medio por consulta · everything: {}", fmt(total_ev / QUERIES.len() as u32));
        println!("(\"rayo ×\" = cuántas veces más rápido que Everything; incluye la ida y vuelta del IPC)");
    }

    // Lista completa ordenada por columna (lo que pasa al tocar un encabezado).
    println!("
Orden completo de todos los resultados:");
    let refs: Vec<&VolumeIndex> = vols.iter().collect();
    let t = Instant::now();
    let ranks: Vec<Option<std::sync::Arc<rayo::index::Ranks>>> = vols.iter().map(|v| Some(std::sync::Arc::new(rayo::index::Ranks::compute(v)))).collect();
    println!("  (órdenes por nombre y ruta precalculados en {}, una sola vez al cargar)", fmt(t.elapsed()));
    for qs in ["a", "config", "ext:jpg;png"] {
        let q = search::parse_query(qs);
        let mut line = format!("  {qs:<14}");
        for (label, sort) in [("relevancia", ResultSort::Relevance), ("nombre", ResultSort::Name), ("carpeta", ResultSort::Folder), ("tamaño", ResultSort::Size), ("fecha", ResultSort::Modified)] {
            let mut times = Vec::new();
            let mut n = 0;
            for _ in 0..5 {
                let t = Instant::now();
                let c = Compiled::new(&q);
                let mut hits: Vec<Hit> = Vec::new();
                for (k, v) in vols.iter().enumerate() {
                    hits.extend(search::search_volume(v, &c, k as u8, usize::MAX).0);
                }
                if sort == ResultSort::Relevance {
                    search::rank(&mut hits, usize::MAX);
                } else {
                    rayo::index::sort_hits(&mut hits, &refs, &ranks, sort, false);
                }
                n = hits.len();
                times.push(t.elapsed());
            }
            line.push_str(&format!(" {label} {}", fmt(median(times))));
            let _ = n;
        }
        println!("{line}");
    }

    // Muestra de resultados para verificar a ojo.
    let q = search::parse_query("config");
    let c = Compiled::new(&q);
    let mut hits = Vec::new();
    for (k, v) in vols.iter().enumerate() {
        hits.extend(search::search_volume(v, &c, k as u8, 5).0);
    }
    search::rank(&mut hits, 5);
    println!("\nTop 5 de 'config':");
    for h in hits {
        println!("  {:>5}  {}", h.score, vols[h.vol as usize].full_path(h.id));
    }
}

/// Prueba de punta a punta del servicio instalado: índice por la tubería y
/// latencia de un cambio real (crear y borrar un archivo) vía USN Journal.
fn service_test() {
    use rayo::index::service::{Changes, Client};
    use rayo::index::usn::{Applier, REASON_CREATE, REASON_DELETE};
    let Some(mut c) = Client::connect() else {
        println!("El servicio no responde. Diagnóstico:
{}", rayo::index::service::diagnose());
        return;
    };
    println!("Servicio conectado.");
    let t = Instant::now();
    let mut v = match c.build('C') {
        Ok(v) => v,
        Err(e) => {
            println!("Error al pedir el índice: {e}");
            return;
        }
    };
    println!("Índice de C: por el servicio: {} entradas en {} (journal {:x}, USN {})", v.live_len(), fmt(t.elapsed()), v.usn_journal, v.usn_next);
    let dir = std::env::temp_dir();
    let name = format!("rayo-prueba-usn-{}.txt", std::process::id());
    let path = dir.join(&name);
    // Ponerse al día antes de crear el archivo.
    let mut applier = Applier::default();
    loop {
        match c.changes('C', v.usn_journal, v.usn_next, 0) {
            Ok(Changes::Batch(list, next)) => {
                for ch in &list {
                    applier.apply(&mut v, ch);
                }
                v.usn_next = next;
                if list.is_empty() {
                    break;
                }
            }
            other => {
                println!("No se pudo leer el journal: {}", if let Err(e) = other { e } else { "reinicio".into() });
                return;
            }
        }
    }
    for (label, create) in [("crear", true), ("borrar", false)] {
        let t = Instant::now();
        if create {
            std::fs::write(&path, b"hola").unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        let want = if create { REASON_CREATE } else { REASON_DELETE };
        let mut seen = None;
        while seen.is_none() && t.elapsed() < Duration::from_secs(10) {
            match c.changes('C', v.usn_journal, v.usn_next, 5) {
                Ok(Changes::Batch(list, next)) => {
                    for ch in &list {
                        applier.apply(&mut v, ch);
                        if ch.name == name && ch.reason & want != 0 {
                            seen = Some(t.elapsed());
                        }
                    }
                    v.usn_next = next;
                }
                _ => break,
            }
        }
        let found = v.find_path(&path.to_string_lossy()).is_some();
        match seen {
            Some(d) => println!("  {label} un archivo: el cambio llegó en {} · en el índice: {}", fmt(d), if found { "sí" } else { "no" }),
            None => println!("  {label} un archivo: el cambio no llegó"),
        }
    }
}

// ---------------------------------------------------------------- copias

/// Tamaño total y cantidad de archivos de una carpeta (o de un archivo).
fn tree_size(p: &std::path::Path) -> (u64, usize) {
    let Ok(m) = std::fs::symlink_metadata(p) else { return (0, 0) };
    if !m.is_dir() {
        return (m.len(), 1);
    }
    let mut acc = (0, 0);
    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
        let (b, n) = tree_size(&e.path());
        acc.0 += b;
        acc.1 += n;
    }
    acc
}

/// El motor de copia del Explorador (el mismo que usa al copiar y pegar), sin ventanas.
fn explorer_copy(src: &str, dest_dir: &str) -> Result<(), String> {
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Shell::*;
    use windows::core::{HSTRING, PCWSTR};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(|e| e.message())?;
        op.SetOperationFlags(FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_NOCONFIRMMKDIR).map_err(|e| e.message())?;
        let s: IShellItem = SHCreateItemFromParsingName(PCWSTR(HSTRING::from(src).as_ptr()), None).map_err(|e| e.message())?;
        let d: IShellItem = SHCreateItemFromParsingName(PCWSTR(HSTRING::from(dest_dir).as_ptr()), None).map_err(|e| e.message())?;
        op.CopyItem(&s, &d, PCWSTR::null(), None).map_err(|e| e.message())?;
        op.PerformOperations().map_err(|e| e.message())
    }
}

fn rayo_copy(src: &str, dest_dir: &str) -> Result<usize, String> {
    use rayo::fsx::transfer::{Op, Transfers};
    let t = Transfers::new(std::sync::Arc::new(|| {}), std::sync::Arc::new(|_| {}));
    let id = t.enqueue(Op::Copy, vec![src.to_string()], dest_dir.to_string());
    loop {
        let v = t.snapshot().into_iter().find(|j| j.id == id).unwrap();
        if v.finished() {
            return if v.errors.is_empty() { Ok(v.workers) } else { Err(v.errors.join("; ")) };
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn copy_bench(src: &str, dest_root: &str, runs: usize) {
    let src_path = std::path::Path::new(src);
    let name = src_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (bytes, files) = tree_size(src_path);
    let vs = rayo::fsx::disks::volume(src);
    let vd = rayo::fsx::disks::volume(dest_root);
    println!("{src} ({} archivos, {:.1} MB) -> {dest_root}   [{:?} -> {:?}]", files, bytes as f64 / 1e6, vs.kind, vd.kind);
    let dirs = [format!("{dest_root}\\explorador"), format!("{dest_root}\\rayo")];
    let mut times: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let mut workers = 1;
    for k in 0..runs {
        let order = if k % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            // RAYO_BENCH_PAUSA=s: esperar a que Windows termine de escribir lo anterior al disco.
            if let Some(p) = std::env::var("RAYO_BENCH_PAUSA").ok().and_then(|v| v.parse::<u64>().ok()) {
                std::thread::sleep(Duration::from_secs(p));
            }
            let dir = &dirs[which];
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(dir).unwrap();
            let t = Instant::now();
            let r = if which == 0 { explorer_copy(src, dir).map(|_| 1) } else { rayo_copy(src, dir) };
            let secs = t.elapsed().as_secs_f64();
            match r {
                Ok(w) => {
                    if which == 1 {
                        workers = w;
                    }
                }
                Err(e) => println!("  error ({}): {e}", ["explorador", "rayo"][which]),
            }
            let copied = tree_size(&std::path::Path::new(dir).join(&name));
            if copied != (bytes, files) {
                println!("  ¡copia incompleta! {:?} de {:?}", copied, (bytes, files));
            }
            times[which].push(secs);
            println!("  vuelta {} {:<10} {:>7.2} s  {:>7.1} MB/s", k + 1, ["explorador", "rayo"][which], secs, bytes as f64 / 1e6 / secs);
        }
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (e, r) = (med(&mut times[0]), med(&mut times[1]));
    println!("  mediana: explorador {e:.2} s · rayo {r:.2} s ({workers} a la vez) → {:.2}×", e / r);
    for d in &dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}
