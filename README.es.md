# Rayo

[English](README.md) · **Español**

Explorador de archivos nativo para Windows: rápido, ligero y con búsqueda instantánea.
Inspirado en la vista de columnas de Finder, OneCommander y Directory Opus, con una interfaz más limpia.

## Qué tiene

- **Vista de columnas** (estilo Finder): cada carpeta abre una columna a la derecha.
- **Vista de cuadrícula** con miniaturas (fotos, videos, PDF).
- **Vista dividida en dos paneles** (`Ctrl+\`), cada uno con sus pestañas; arrastrar y soltar entre ellos
  (mismo disco = mover, otro disco = copiar; Ctrl fuerza copiar y Mayús fuerza mover).
- **Cola de copias propia**: progreso, velocidad, tiempo restante, pausa, cancelar y conflictos
  (reemplazar, omitir, conservar ambos). Mover en el mismo disco es instantáneo.
- **Paleta de comandos** (Ctrl+K): todas las acciones con búsqueda difusa sin acentos; escribir una ruta la abre.
  El menú de clic derecho sale del mismo catálogo, ordenado en submenús (Otro panel, Herramientas, Ver).
- **Un solo menú de clic derecho, con todo**: las opciones de Rayo y, debajo, las de Windows y tus apps (7-Zip,
  VLC, Git, «Abrir con», «Enviar a»…) con sus íconos y submenús, sin duplicados. Arriba, una búsqueda que filtra
  todas las opciones (también las de submenús; Enter ejecuta la primera) y una estrella para fijar las más usadas.
  Con 7-Zip instalado se usa el suyo para extraer y comprimir. El menú y sus submenús son ventanas propias:
  salen por fuera de Rayo aunque la ventana sea chica, y se acomodan a los bordes del monitor. Las opciones de
  Windows que no uses se ocultan con el ojo tachado (abajo, «N opciones ocultas: mostrar» para recuperarlas).
- **Renombrado en lote con vista previa** (Ctrl+Mayús+R): plantilla con `{nombre}`, `{n}`, `{fecha}`,
  `{exif}` (fecha de la foto), `{carpeta}`; buscar y reemplazar (también con expresiones regulares),
  mayúsculas, numeración y extensión. Detecta nombres repetidos o inválidos antes de tocar nada
  y permite intercambiar nombres (a↔b) sin choques.
- **Buscador de duplicados**: candidatos por tamaño desde el índice (instantáneo), luego los primeros 16 KB
  y por último el contenido completo con xxh3. Ignora enlaces duros, nunca deja un grupo sin copia
  y manda lo marcado a la papelera.
- **Qué ocupa espacio**: detalles ordenados por tamaño (carpetas y archivos mezclados) con barras de uso.
- **Comparar y sincronizar carpetas**: ambas direcciones, actualizar o espejo; acción por elemento con un clic;
  las copias van por la cola y al terminar se vuelve a comparar.
- **Comprimidos como carpetas**: ZIP con lector propio; 7z, RAR, TAR, ISO y otros con el 7-Zip instalado.
  Vista previa de lo de adentro, abrir un archivo (copia temporal), arrastrar fuera para extraer,
  **Extraer aquí**, **Extraer en una carpeta** y **Comprimir en ZIP**, todo por la cola de copias.
- **Deshacer** (Ctrl+Z): renombrar, renombrado en lote, papelera (restaura desde la papelera de Windows),
  copias, movimientos, extracciones, ZIP creados y carpetas nuevas.
- **Arrastrar con otras apps**: soltar en Rayo archivos del Explorador, el navegador, etc., y sacar archivos de
  Rayo hacia cualquier app (con el arrastre nativo de Windows). Arrastrar sobre una pestaña la abre;
  soltar sobre ella lleva los archivos a su carpeta.
- **Selección con rectángulo** arrastrando desde un espacio vacío (en detalles, también desde fecha, tipo o tamaño);
  clic en vacío deselecciona.
- **Galería instantánea, también con fotos pesadas en discos mecánicos**: cada miniatura sale de la caché de
  Windows o de la miniatura que guardó la cámara dentro de la foto (~10 ms, sin leer los 20+ MB del archivo); solo
  si hace falta más tamaño se lee la foto entera. La vista previa y la barra espaciadora tienen su propio hilo y
  nunca esperan detrás de la galería.
- **Vista previa de casi todo**: imágenes, **PDF** con su página real (y ← → para pasar páginas en la vista rápida),
  texto y código, **Markdown con formato**, y los visores de Windows (Word, Excel, PowerPoint, RTF, fuentes, audio…)
  dentro del panel, como en el Explorador.
- **OneDrive sin sorpresas**: los archivos que están solo en la nube nunca se descargan por mirarlos (miniaturas,
  vista previa, duplicados); un ícono muestra si están en la nube, descargados o siempre en el equipo.
- **Nuevo ▸** (clic derecho): carpeta, documento de texto, Markdown, ZIP y los tipos que registren tus programas
  (Word, Excel…), con el nombre listo para escribir.
- **Barra de ruta con autocompletado**: ↑ ↓ para elegir, Tab para completar.
  - **Soltar archivos sobre un tramo** de la ruta los mueve a esa carpeta (Ctrl copia).
  - **Arrastrar un tramo** lleva esa carpeta a otra app, al otro panel o a otra carpeta; la pestaña la sigue.
  - **▸ entre tramos**: las carpetas hermanas, con la actual marcada; «…» muestra las que no entran.
  - **Comandos**: `cmd`, `powershell`, `code .` o cualquier programa se abren en la carpeta actual;
    `~` es tu carpeta personal, `%APPDATA%` y demás variables funcionan, y `..` o un nombre suelto
    navegan desde la carpeta actual.
- **Se instala solo**: el mismo `rayo.exe` (un solo archivo, sin DLLs propias) ofrece la primera vez «Instalar»
  (en `%LOCALAPPDATA%\Programs\Rayo`, con acceso en el menú Inicio y desinstalador en «Aplicaciones instaladas»,
  sin permisos de administrador) o «Usar sin instalar» (portátil). La paleta tiene «Instalar Rayo (o actualizarlo
  con esta versión)».
- **Continuar como administrador**: si Windows niega borrar, copiar, mover, renombrar o crear algo por permisos,
  Rayo ofrece repetir solo esa operación con el aviso de Windows; la app sigue con permisos normales.
- **Sin barra de título**: las pestañas van arriba, en la misma fila que minimizar, maximizar y cerrar (como el
  Explorador de Windows 11). Se arrastra desde el espacio libre de esa fila, doble clic maximiza, y siguen los
  bordes, la sombra, Aero Snap y el menú de Snap Layouts sobre maximizar.
- **Explorador predeterminado** (paleta: «Usar Rayo como explorador predeterminado…»): las carpetas y .zip que abras
  desde cualquier programa, Win+E, «Mostrar en carpeta» del navegador y «Abrir ubicación del archivo» se abren
  en Rayo. Solo cambia tu usuario, sin administrador; se apaga desde el mismo lugar y queda
  `%APPDATA%\Rayo\restaurar-explorador.reg` para deshacerlo a mano. Mayús al abrir algo usa el Explorador esa vez.
  La ventana del Explorador que abren Win+E o «Mostrar en carpeta» se saca de la pantalla antes de dibujarse:
  solo se ve Rayo.
- **En español e inglés, y traducible por cualquiera**: Ctrl+K → «Idioma…» cambia el idioma al instante
  (o «igual que Windows»). Cada idioma es un CSV en `idiomas/` (clave, texto, original en español y una nota
  para el traductor); para otro idioma basta un `pt.csv` en `%APPDATA%\Rayo\idiomas`, sin recompilar, y lo que
  falte se muestra en inglés. Guía: [idiomas/LÉEME.md](idiomas/LÉEME.md).
- **Historial de búsquedas**: con la caja vacía, ↓ muestra las últimas (Enter para repetir, X para quitar).
- **¿Quién lo está usando?** (clic derecho): qué programa tiene abierto un archivo o algo de una carpeta,
  con botones para cerrarlo. Si algo no se deja borrar o renombrar, se abre solo y permite reintentar.
- **Seleccionar por patrón** (`*.jpg; *.png`, Ctrl+Mayús+S) e **invertir selección** (Ctrl+I).
- **Papelera** dentro de Rayo: ver, filtrar, restaurar, eliminar definitivamente y vaciar.
- **Recientes**: lo modificado en la semana en todo el equipo, sin el ruido del sistema (`recientes:30d` para otro período).
- **Vista plana** (Ctrl+Mayús+L): todos los archivos de la carpeta y sus subcarpetas en una lista.
- **Vista y orden por carpeta**: cada carpeta recuerda los suyos; las demás usan detalles por nombre
  (cambiable con «Usar esta vista en todas las carpetas»).
- **Hash** SHA-256, SHA-1 y MD5 con comparación, y **enlaces** («Pegar como enlace», «Crear enlace en el otro panel»).
- Clic derecho en lugares, unidades y favoritos: abrir en pestaña nueva, en el otro panel, terminal, propiedades.
- **Filtro al escribir**: con la lista enfocada, escribir filtra la carpeta (sin mayúsculas ni acentos); Esc lo quita.
- **Rutas de más de 260 caracteres** en listado, copias, renombrado e índice.
- **Vista de detalles**: fecha, **antigüedad con colores** (rojo = nuevo → azul → gris = viejo), tipo y tamaño; se ordena clicando los encabezados.
- **Búsqueda instantánea** en todo el equipo mientras escribes (índice propio, ver abajo).
  No distingue mayúsculas ni acentos: «cancion» encuentra «Canción».
  Muestra primero los mejores al instante; con «ver todos» (o al llegar al final) trae la lista completa,
  y se puede ordenar por nombre, carpeta, tamaño o fecha (1,2 millones de resultados en ~25 ms).
- **Índice siempre al día con el USN Journal**: con el servicio activado, Rayo abre sin reindexar
  (solo lee los cambios ocurridos mientras estuvo cerrado) y ve cada cambio del disco al instante.
- **Una sola instancia**: abrir Rayo otra vez enfoca la ventana existente; `rayo.exe "C:\ruta"` la abre en una pestaña.
- **Abre al instante**: al cerrar la ventana, Rayo queda en segundo plano (como el Explorador, que siempre está
  corriendo) con un ícono en la bandeja; volver a abrirlo tarda ~12 ms. Ctrl+Q, «Salir de Rayo» en la bandeja
  o `rayo.exe --quit` lo cierran del todo. Se desactiva con «Quedarse en segundo plano al cerrar» (Ctrl+K).
  Opcional: «Iniciar con Windows» lo deja listo, oculto, desde que inicias sesión.
- **Tamaño de las carpetas** al instante, calculado desde el índice.
- **Pestañas**, **grupos de favoritos**, **unidades** con barra de uso y botón para **expulsar**.
- **Vista previa** (imágenes con WIC, miniaturas de video/PDF del shell, texto y código) y **Quick Look** con la barra espaciadora.
- **Menú contextual** propio y el **menú nativo de Windows** completo (7-Zip, Git, etc.).
- Copiar, cortar y pegar compatible con el Explorador, papelera, renombrar en línea, nueva carpeta.
- Tema oscuro y claro. Las carpetas visibles se actualizan solas cuando algo cambia en disco.

## Atajos

| Tecla | Acción |
|---|---|
| ↑ ↓ ← → | Moverse (en columnas, → entra y ← sale) |
| Enter / doble clic | Abrir |
| Espacio | Quick Look |
| Escribir letras | Filtrar la carpeta actual (Esc lo quita) |
| Ctrl+K o Ctrl+Mayús+P | Paleta de comandos |
| `Ctrl+\` / Tab | Dividir en dos paneles / pasar al otro panel |
| Ctrl+Mayús+R | Renombrar en lote |
| Ctrl+F | Buscar |
| Ctrl+Enter (en resultados) | Mostrar en su carpeta |
| Ctrl+L | Escribir una ruta |
| Ctrl+T / Ctrl+W / Ctrl+Tab | Pestaña nueva / cerrar / siguiente |
| Ctrl+1 / Ctrl+2 / Ctrl+3 | Vista de columnas / detalles / cuadrícula |
| Ctrl+P | Panel de vista previa |
| Ctrl+H | Mostrar archivos ocultos |
| F2 | Renombrar |
| Ctrl+Z | Deshacer |
| Ctrl+Mayús+S / Ctrl+I | Seleccionar por patrón / invertir selección |
| Ctrl+Mayús+L | Vista plana (todas las subcarpetas) |
| Supr / Mayús+Supr | Papelera / borrar definitivamente |
| Ctrl+C / Ctrl+X / Ctrl+V | Copiar / cortar / pegar |
| Ctrl+Mayús+C | Copiar ruta |
| Ctrl+Mayús+N | Nueva carpeta |
| Alt+← / Alt+→ / Alt+↑ / Retroceso | Atrás / adelante / subir |
| Alt+Enter | Propiedades |
| Mayús+F10 o Mayús+clic derecho | Menú nativo de Windows |
| F5 | Actualizar |
| F12 | Monitor de rendimiento (cuadros/s y CPU por cuadro) |
| Ctrl+Q | Salir de Rayo (no quedarse en segundo plano) |

Sintaxis de búsqueda: `ext:pdf;docx`, `size:>1gb`, `size:10mb-1gb`, `dm:hoy` (`ayer`, `semana`, `mes`, `año`, `7d`, `3h`),
`tipo:imagen` (`video`, `audio`, `doc`, `zip`, `codigo`, `exe`), `carpeta:` / `archivo:`, `!excluir`, `"frase exacta"`,
comodines `*.mp4`, y rutas `documentos\ factura`.

## Cómo funciona

- **Rust + egui (OpenGL)**. Sin runtime ni dependencias externas: un solo `.exe` de ~8 MB.
  Solo dibuja cuando algo cambia, así que en reposo usa 0 % de CPU.
- **Índice**: todos los nombres de un disco viven en un único buffer contiguo, en minúsculas y sin acentos.
  Las mayúsculas se guardan en un bitmap y los acentos como parches de 4 bytes, así que el texto no se duplica.
  Una búsqueda es un barrido SIMD (`memchr`; Aho-Corasick para muchas extensiones) partido en trozos
  que se procesan en paralelo; tras cada coincidencia salta al siguiente nombre y cada hilo conserva
  solo los mejores resultados. Los filtros `size:` y `dm:` recorren primero los arreglos numéricos.
- **Indexado**: con permisos de administrador lee la **MFT** de NTFS en crudo; sin ellos, recorre las carpetas
  en paralelo con `FindFirstFileExW`. Corre en un **proceso aparte** (`rayo.exe --build-index C`) para que su memoria
  temporal se libere completa al terminar; la app carga el resultado desde la caché (`%LOCALAPPDATA%\Rayo`).
- **Al día**:
  - **Con el servicio** (botón «Activar índice instantáneo» en la barra de estado; pide administrador una sola vez):
    un servicio de Windows (`RayoIndexer`, en `C:\Program Files\Rayo`) lee la MFT y el **USN Journal** y se los pasa
    a la app por una named pipe. La app guarda el índice al cerrar con su posición en el journal; al abrir aplica solo
    los cambios desde entonces, y mientras está abierta espera cambios nuevos sin sondear (0 % de CPU en reposo).
    Renombrar o mover carpetas conserva todo su contenido. Es el mismo reparto que usa Everything.
    La app verifica que la tubería pertenezca al servicio (su PID debe coincidir con el que informa Windows para
    `RayoIndexer`, nombre que solo un administrador puede registrar). Registro del servicio: `C:\ProgramData\Rayo\servicio.log`.
  - **Sin el servicio**: recorre las carpetas al abrir y escucha cambios con `ReadDirectoryChangesW`.
- **Orden de resultados**: al cargar el índice se precalcula la posición de cada archivo por nombre y por ruta
  (~60 ms, ~13 MB), así que ordenar una lista enorme es comparar números.

Código: `src/index` (índice y búsqueda), `src/fsx` (Windows: listados, shell, miniaturas), `src/ui` (interfaz).

## Medidas en esta PC (Ryzen 7 9800X3D, 1,66 millones de archivos en C:)

| | |
|---|---|
| Abrir en frío (ventana visible, con la carpeta) | ~305 ms: ~250 de OpenGL (driver), ~20 crear la ventana, ~20 mostrarla |
| Volver a abrir (en segundo plano) | ~12 ms |
| Explorador de Windows (mismo método: `explorer.exe ruta`) | ~600 ms |
| En segundo plano | 0 % de CPU, ~17 MB de RAM en uso |
| Índice listo (desde caché) | ~285 ms después de abrir |
| Indexado completo | 1,16 s leyendo la MFT (admin); 1,5–7 s recorriendo carpetas (sin admin) |
| Memoria | ~180 MB en total: ~80 interfaz + ~99 índice |
| CPU por cuadro | ~0,4 ms (presupuesto a 144 Hz: 6,9 ms) |
| Búsqueda típica | 0,13–0,3 ms; promedio 0,69 ms; una sola letra ~4 ms |

Contra **Everything 1.4.1** (mismas 17 consultas, top 1000, medido por su IPC): Rayo gana en todas.
Promedio 0,69 ms contra 7,1 ms; búsquedas de texto 30–60× más rápido, por ruta ~150×,
`ext:`, `size:`, `dm:` y comodines 2–10×, una sola letra ~1,4×. Con la MFT, los conteos coinciden con los de Everything.

## Compilar y medir

```bash
cargo build --release
```

```bash
cargo run --release --bin rayo-bench -- --cache
```

`rayo-bench` compara un motor ingenuo, el de Rayo y **Everything** (si está abierto, por su IPC).
Con `--mft`, en una terminal de administrador, prueba el lector de la MFT.

Con el servicio instalado, `rayo-bench --servicio` prueba todo de punta a punta: pide el índice al servicio
y mide cuánto tarda en llegar el cambio al crear y borrar un archivo.

Opciones de `rayo.exe`: `--quit` (cerrar del todo el Rayo abierto), `--background` (arrancar oculto),
`--install-service` / `--uninstall-service` (como administrador), `--service` (lo usa Windows),
`--build-index C` (indexador en proceso aparte).

`vendor/winit` es winit 0.30.13 con una sola función agregada (`prepare_keyboard_layout`): winit arma su tabla
del teclado la primera vez que la ventana recibe el foco (~70 ms en el hilo de la interfaz, justo al mostrarse);
Rayo la prepara en otro hilo mientras se inicializa OpenGL.

**Textos de la interfaz:** nunca escritos en el código, sino `t!("clave")`, `tf!("clave", n = 3)` y
`tp!(cantidad, "clave")` (plural: `clave.uno` / `clave.varios`), con el texto en `idiomas/es.csv` e
`idiomas/en.csv`. `cargo test` falla si una clave del código falta en alguno de los dos o si sus `{variables}`
no coinciden; `rayo.exe --revisar-idiomas` revisa los CSV de las carpetas de idiomas.

Variables de diagnóstico: `RAYO_DEBUG=1` (registro de memoria y tiempos en `%LOCALAPPDATA%\Rayo\debug.log`),
`RAYO_NO_INDEX=1`, `RAYO_NO_REBUILD=1`. Si Rayo se cae, qué pasó y en qué línea queda al final de
`%LOCALAPPDATA%\Rayo\errores.log`.
