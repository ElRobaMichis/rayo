# Traducir Rayo

Cada idioma es un archivo CSV en esta carpeta: `es.csv` (español, el original) y `en.csv` (inglés).
Los dos van dentro de `rayo.exe`; cualquier otro idioma se agrega sin recompilar.

## Columnas

| columna | qué es |
|---|---|
| `clave` | el nombre fijo del texto (`menu.copiar_ruta`). **No se cambia.** |
| `texto` | la traducción. Es lo único que se completa. |
| `original (es)` | el texto en español, para traducir viendo el original. |
| `nota` | dónde aparece o qué significa cada `{variable}`. |

## Paso a paso

1. En Rayo: **Ctrl+K → «Idioma…» → «Abrir la carpeta de idiomas»**. Ahí aparece `_plantilla.csv`
   (en `%APPDATA%\Rayo\idiomas`), con cada texto en español y en inglés y la columna `texto` vacía.
2. Cópiala con el código de tu idioma: `pt.csv` (portugués), `fr.csv` (francés), `de.csv` (alemán)…
3. Ábrela con Excel o Google Sheets y completa la columna `texto`. Empieza por `idioma.nombre`:
   el nombre del idioma en ese idioma («Português»), que es lo que se ve en la lista.
4. Guárdala como **CSV UTF-8** (en Excel: «Guardar como → CSV UTF-8 (delimitado por comas)»).
   Si Excel la guarda con punto y coma, también sirve.
5. En Rayo, Ctrl+K → «Idioma…» y elígelo: se aplica al instante, sin reiniciar.
6. **«Revisar traducciones»** dice cuánto está traducido, qué falta y si alguna `{variable}` quedó distinta.
   Desde una terminal: `rayo.exe --revisar-idiomas`.

Lo que falte se muestra en inglés, así que una traducción a medias ya se puede usar.

## Reglas

- **Las `{variables}` se dejan tal cual** (`{n}`, `{nombre}`, `{error}`…): Rayo pone ahí el número, el
  nombre o el error. Se pueden mover de lugar, pero no traducir ni borrar.
- **Plurales:** las claves que terminan en `.uno` se usan cuando la cantidad es 1 y las `.varios` en
  cualquier otro caso («1 elemento» / «3 elementos»).
- **Atajos y teclas:** `tecla.mayus`, `tecla.supr`… son los nombres de las teclas en tu teclado (Shift, Del).
- **Comandos:** `cmd.<id>` es el nombre del comando y `cmd.<id>.buscar` son palabras extra para
  encontrarlo en la paleta (sin acentos, separadas por espacios).
- **Plantilla de renombrar:** `plantilla.*` son las piezas que se escriben entre llaves (`{nombre}`):
  una sola palabra. Las de español e inglés funcionan siempre.
- Los textos del menú de Windows (7-Zip, «Enviar a»…) no están acá: los pone Windows en su idioma.

## Para mandar una traducción

Manda el CSV (por ejemplo `pt.csv`). Para que venga dentro de `rayo.exe`, se copia a esta carpeta del
proyecto y se agrega en `src/i18n.rs` igual que `ES` y `EN` (`include_str!` y su caso en `sources`).

---

# Translating Rayo

Each language is a CSV file in this folder. Copy `_plantilla.csv` (Ctrl+K → “Language…” → “Open the
languages folder”) to your language code (`pt.csv`, `fr.csv`…), fill in the `texto` column (start with
`idioma.nombre`, your language's name in your language), save it as **CSV UTF-8**, and pick it in
Ctrl+K → “Language…”. Keep every `{variable}` as is; `.uno` keys are used when the count is 1 and
`.varios` otherwise. “Check translations” (or `rayo.exe --revisar-idiomas`) lists what's missing.
