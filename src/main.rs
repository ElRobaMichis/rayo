#![windows_subsystem = "windows"]

use rayo::tf;
use eframe::egui;

/// Ícono de la ventana mientras arranca (lo dibuja `build.rs`, el mismo que lleva el .exe).
fn icon() -> egui::IconData {
    let rgba = include_bytes!(concat!(env!("OUT_DIR"), "/icon64.rgba")).to_vec();
    egui::IconData { rgba, width: 64, height: 64 }
}

/// Si Rayo se cae, deja qué pasó y dónde en `%LOCALAPPDATA%\Rayo\errores.log` (se agrega al final),
/// para poder mandarlo: con `panic = "abort"` la ventana desaparece sin decir nada.
fn log_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            let dir = std::path::Path::new(&base).join("Rayo");
            let _ = std::fs::create_dir_all(&dir);
            let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
            let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let line = format!(
                "[{}] Rayo {} · hilo «{}» · {at}\n    {msg}\n    argumentos: {:?}\n\n",
                rayo::fsx::fmt_date(secs as u32),
                env!("CARGO_PKG_VERSION"),
                std::thread::current().name().unwrap_or("?"),
                std::env::args().skip(1).collect::<Vec<_>>()
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("errores.log")) {
                let _ = f.write_all(line.as_bytes());
            }
        }
        default(info);
    }));
}

/// Muestra el error (si lo hay) en un cuadro de diálogo y devuelve el código de salida.
fn report(r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            let text: Vec<u16> = tf!("inicio.no_pudo_completar_operacion", error = e).encode_utf16().chain([0]).collect();
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    None,
                    windows::core::PCWSTR(text.as_ptr()),
                    windows::core::w!("Rayo"),
                    windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
            1
        }
    }
}

fn main() -> eframe::Result {
    let _ = rayo::index::START.set(std::time::Instant::now());
    log_crashes();
    // Modo indexador: lo lanza la propia app en un proceso aparte, para que la
    // memoria temporal del indexado se libere por completo al terminar.
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--build-index") => {
            let letter = args.get(2).and_then(|s| s.chars().next()).unwrap_or('C');
            std::process::exit(rayo::index::build_child(letter));
        }
        // Revisar las traducciones (los CSV de las carpetas de idiomas) y mostrar qué falta.
        Some("--revisar-idiomas") => {
            unsafe {
                let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
            }
            println!("\n{}", rayo::i18n::check_all());
            std::process::exit(0);
        }
        // Lo lanza el administrador de servicios de Windows.
        Some("--service") => std::process::exit(rayo::index::service::run_as_service()),
        // Instalar / quitar el servicio (la app los lanza con permisos de administrador).
        Some("--install-service") => std::process::exit(report(rayo::index::service::install())),
        Some("--uninstall-service") => std::process::exit(report(rayo::index::service::uninstall())),
        // Una operación con permisos de administrador ("Continuar como administrador").
        Some("--admin-op") => std::process::exit(rayo::admin::execute(args.get(2).map(String::as_str).unwrap_or(""))),
        // Instalar o actualizar sin preguntar (para desarrollo): cierra el Rayo abierto,
        // copia esta versión a la instalación y la abre.
        Some("--instalar") => {
            rayo::instance::quit_running();
            std::thread::sleep(std::time::Duration::from_millis(800));
            let r = rayo::install::install().and_then(|_| rayo::install::launch_installed());
            std::process::exit(report(r));
        }
        // Desinstalar (lo lanza "Aplicaciones instaladas" de Windows).
        Some("--uninstall") => std::process::exit(rayo::install::uninstall()),
        // Cerrar del todo el Rayo abierto (también si está en segundo plano).
        Some("--quit") => std::process::exit(if rayo::instance::quit_running() { 0 } else { 1 }),
        _ => {}
    }
    // Recién instalado: esperar a que se cierre el Rayo que lo instaló.
    rayo::install::wait_for_previous(&args);
    // Arranque oculto (inicio de sesión): si ya hay uno abierto, no hacer nada.
    if args.iter().any(|a| a == "--background") {
        if rayo::instance::is_running() {
            return Ok(());
        }
        rayo::instance::START_HIDDEN.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // Si ya hay un Rayo abierto, se le pasa la ruta y se termina aquí.
    if !rayo::instance::claim(rayo::instance::path_from_args().as_deref()) {
        return Ok(());
    }
    // Mientras se inicializa OpenGL (~250 ms), preparar en otro hilo la tabla del teclado
    // que winit necesita al mostrar la ventana (si no, la arma ahí mismo: ~70 ms).
    std::thread::spawn(winit::platform::windows::prepare_keyboard_layout);
    rayo::index::mem_log("main: creando ventana");

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rayo")
            .with_app_id("rayo")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([760.0, 440.0])
            .with_icon(icon()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("Rayo", opts, Box::new(|cc| Ok(Box::new(rayo::ui::App::new(cc)))))
}
