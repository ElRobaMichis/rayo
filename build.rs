//! Ícono de Rayo: se dibuja aquí (una carpeta azul con un rayo), se guarda como
//! .ico con todos los tamaños de Windows y se incrusta en el .exe. Así lo ven la
//! barra de tareas (también anclado), el Explorador, los accesos directos y Alt+Tab.

use std::path::{Path, PathBuf};

/// Tamaños que pide Windows (escalas 100 % a 300 % y la vista de íconos grandes).
const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 96, 256];

type Poly = Vec<(f32, f32)>;

/// Cuánto se agranda el dibujo (diseñado en una grilla de 256) dentro del cuadro.
const GROW: f32 = 1.20;

/// Esquina redondeada: arco de centro (cx, cy) entre dos ángulos (grados).
fn arc(p: &mut Poly, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
    let n = 24;
    for i in 0..=n {
        let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
        p.push((cx + r * a.cos(), cy + r * a.sin()));
    }
}

fn rounded_rect(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Poly {
    let mut p = Vec::new();
    arc(&mut p, x0 + r, y0 + r, r, 180.0, 270.0);
    arc(&mut p, x1 - r, y0 + r, r, 270.0, 360.0);
    arc(&mut p, x1 - r, y1 - r, r, 0.0, 90.0);
    arc(&mut p, x0 + r, y1 - r, r, 90.0, 180.0);
    p
}

/// Fondo de la carpeta con su pestaña (en una grilla de 256).
fn folder_back() -> Poly {
    let r = 24.0;
    let mut p = Vec::new();
    arc(&mut p, 24.0 + r, 40.0 + r, r, 180.0, 270.0);
    // Pestaña: recta y bajada en diagonal hasta el borde superior.
    p.push((98.0, 40.0));
    p.push((106.0, 42.0));
    p.push((117.0, 49.0));
    p.push((128.0, 62.0));
    arc(&mut p, 232.0 - r, 62.0 + r, r, 270.0, 360.0);
    arc(&mut p, 232.0 - r, 220.0 - r, r, 0.0, 90.0);
    arc(&mut p, 24.0 + r, 220.0 - r, r, 90.0, 180.0);
    p
}

fn bolt() -> Poly {
    vec![(144.0, 108.0), (102.0, 162.0), (127.0, 162.0), (114.0, 204.0), (158.0, 146.0), (133.0, 146.0)]
}

/// Pinta un polígono con antialias (8 subfilas por píxel, cobertura horizontal exacta).
fn fill(img: &mut [f32], size: u32, poly: &Poly, color: [f32; 3]) {
    let k = size as f32 / 256.0;
    // El dibujo se agranda desde su centro para que la carpeta ocupe casi todo el cuadro
    // (si no, se ve chica al lado de los demás íconos de la barra de tareas).
    let grow = |x: f32, y: f32| ((x - 128.0) * GROW + 128.0, (y - 130.0) * GROW + 128.0);
    let pts: Vec<(f32, f32)> = poly.iter().map(|&(x, y)| grow(x, y)).map(|(x, y)| (x * k, y * k)).collect();
    let n = size as usize;
    let sub = 8;
    let mut cov = vec![0f32; n * n];
    for py in 0..n {
        for s in 0..sub {
            let y = py as f32 + (s as f32 + 0.5) / sub as f32;
            let mut xs: Vec<f32> = Vec::new();
            for i in 0..pts.len() {
                let (x0, y0) = pts[i];
                let (x1, y1) = pts[(i + 1) % pts.len()];
                if (y0 <= y) != (y1 <= y) {
                    xs.push(x0 + (y - y0) * (x1 - x0) / (y1 - y0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for span in xs.chunks(2) {
                let [a, b] = span else { continue };
                let (a, b) = (a.max(0.0), b.min(n as f32));
                if b <= a {
                    continue;
                }
                let (ia, ib) = (a.floor() as usize, (b.ceil() as usize).min(n));
                for px in ia..ib {
                    let l = (px as f32).max(a);
                    let r = (px as f32 + 1.0).min(b);
                    if r > l {
                        cov[py * n + px] += (r - l) / sub as f32;
                    }
                }
            }
        }
    }
    // Sobre lo ya pintado (alfa premultiplicado).
    for (i, c) in cov.iter().enumerate() {
        let a = c.min(1.0);
        if a <= 0.0 {
            continue;
        }
        let px = &mut img[i * 4..i * 4 + 4];
        for ch in 0..3 {
            px[ch] = color[ch] * a + px[ch] * (1.0 - a);
        }
        px[3] = a + px[3] * (1.0 - a);
    }
}

fn hex(c: u32) -> [f32; 3] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// RGBA (sin premultiplicar) del ícono a este tamaño.
fn render(size: u32) -> Vec<u8> {
    let mut img = vec![0f32; (size * size * 4) as usize];
    fill(&mut img, size, &folder_back(), hex(0x2F6FED));
    fill(&mut img, size, &rounded_rect(24.0, 92.0, 232.0, 218.0, 24.0), hex(0x4C8DFF));
    fill(&mut img, size, &bolt(), hex(0xFFFFFF));
    img.chunks(4)
        .flat_map(|p| {
            let a = p[3];
            let un = |c: f32| if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            [un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round() as u8]
        })
        .collect()
}

/// .ico con cada tamaño como mapa de bits de 32 bits (BGRA, de abajo hacia arriba).
fn write_ico(path: &Path, images: &[(u32, Vec<u8>)]) {
    let mut dir = Vec::new();
    let mut data = Vec::new();
    dir.extend_from_slice(&[0, 0, 1, 0]);
    dir.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, rgba) in images {
        let mut bmp = Vec::new();
        // BITMAPINFOHEADER: la altura cuenta también la máscara AND.
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(*s as i32).to_le_bytes());
        bmp.extend_from_slice(&((*s * 2) as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 24]);
        for y in (0..*s).rev() {
            for x in 0..*s {
                let i = ((y * s + x) * 4) as usize;
                bmp.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        // Máscara AND en ceros (la transparencia la da el alfa); filas de 4 bytes.
        let row = (s.div_ceil(32) * 4) as usize;
        bmp.extend(std::iter::repeat_n(0u8, row * *s as usize));
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        dir.extend_from_slice(&[dim, dim, 0, 0]);
        dir.extend_from_slice(&1u16.to_le_bytes());
        dir.extend_from_slice(&32u16.to_le_bytes());
        dir.extend_from_slice(&(bmp.len() as u32).to_le_bytes());
        dir.extend_from_slice(&offset.to_le_bytes());
        offset += bmp.len() as u32;
        data.extend(bmp);
    }
    dir.extend(data);
    std::fs::write(path, dir).expect("no se pudo escribir el .ico");
}

/// `rc.exe` (el compilador de recursos de MSVC): el del PATH si está (consola de Visual Studio) y si no
/// el del SDK de Windows más nuevo, que es lo que hay en una PC común o en GitHub Actions.
fn find_rc() -> PathBuf {
    let on_path = std::process::Command::new("rc.exe").arg("/?").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok();
    if on_path {
        return PathBuf::from("rc.exe");
    }
    let arch = if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") { "arm64" } else { "x64" };
    let kits = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)")).join(r"Windows Kits\10\bin");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&kits)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join(arch).join("rc.exe"))
        .filter(|p| p.is_file())
        .collect();
    // Las carpetas se llaman por versión (10.0.22621.0): la más nueva queda última.
    found.sort();
    found.pop().unwrap_or_else(|| PathBuf::from("rc.exe"))
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, render(s))).collect();
    let ico = out.join("rayo.ico");
    write_ico(&ico, &images);
    // El de 64 px, para la ventana mientras arranca.
    std::fs::write(out.join("icon64.rgba"), &images.iter().find(|(s, _)| *s == 64).unwrap().1).unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let rc = out.join("rayo.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "/"))).unwrap();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let res = out.join("rayo.res");
        let ok = std::process::Command::new(find_rc()).arg("/nologo").arg("/fo").arg(&res).arg(&rc).status().is_ok_and(|s| s.success());
        if ok {
            println!("cargo:rustc-link-arg-bin=rayo={}", res.display());
        } else {
            println!("cargo:warning=no se encontró rc.exe: el .exe queda sin ícono");
        }
    } else {
        let obj = out.join("rayo-icon.o");
        let ok = std::process::Command::new("windres").arg(&rc).arg("-O").arg("coff").arg("-o").arg(&obj).status().is_ok_and(|s| s.success());
        if ok {
            println!("cargo:rustc-link-arg-bin=rayo={}", obj.display());
        } else {
            println!("cargo:warning=no se encontró windres: el .exe queda sin ícono");
        }
    }
}
