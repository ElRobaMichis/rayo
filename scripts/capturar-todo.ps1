# Captura todas las partes de la interfaz de Rayo, en tema oscuro y claro, para revisar el diseño.
# No compila: usa un rayo.exe ya compilado (por defecto target\release\rayo.exe). Cada captura es
# una instancia aparte y aislada (ver scripts/captura.ps1); si una falla, se sigue con las demás.
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capturar-todo.ps1 [-exe ruta] [-salida carpeta] [-solo nombre1,nombre2] [-tema oscuro|claro]
param(
    [string]$exe = "$PSScriptRoot\..\target\release\rayo.exe",
    [string]$salida = "$PSScriptRoot\..\target\capturas",
    # Solo estas capturas (por nombre); vacío = todas.
    [string]$solo = "",
    # Solo un tema; vacío = los dos.
    [string]$tema = ""
)

$ErrorActionPreference = "Continue"
$exe = (Resolve-Path $exe).Path
New-Item -ItemType Directory -Force -Path $salida | Out-Null
$salida = (Resolve-Path $salida).Path

# Nombre, pasos de -abrir, vista y espera (ms) antes de capturar. Los pasos corren al segundo
# de arrancar; los que escriben o hacen clic, un poco después (ver src/ui/capture_steps.rs).
$capturas = @(
    @{ n = "detalles"; a = ""; v = "Details" },
    @{ n = "columnas"; a = ""; v = "Columns" },
    @{ n = "grilla"; a = ""; v = "Grid" },
    @{ n = "paleta"; a = "paleta" },
    @{ n = "borrar"; a = "borrar" },
    @{ n = "hash"; a = "hash"; e = 3000 },
    @{ n = "renombrar"; a = "renombrar"; e = 3000 },
    @{ n = "bloqueos"; a = "bloqueos"; e = 4500 },
    @{ n = "conexion"; a = "conexion" },
    @{ n = "barra"; a = "barra" },
    @{ n = "patron"; a = "patron" },
    @{ n = "idioma"; a = "idioma" },
    @{ n = "predeterminado"; a = "predeterminado" },
    @{ n = "admin"; a = "admin" },
    @{ n = "primer-uso"; a = "primer-uso" },
    @{ n = "conflicto"; a = "conflicto"; e = 3000 },
    @{ n = "copias"; a = "copias"; e = 1900 },
    @{ n = "duplicados"; a = "duplicados"; e = 4000 },
    @{ n = "comparar"; a = "comparar"; e = 3000 },
    @{ n = "papelera"; a = "papelera"; e = 3500 },
    @{ n = "terminal"; a = "terminal"; e = 4000 },
    @{ n = "git"; a = "git"; e = 4000 },
    @{ n = "terminal-git"; a = "git,terminal"; e = 4500 },
    @{ n = "cuatro"; a = "cuatro"; e = 3000 },
    @{ n = "dividida"; a = "dividida"; e = 3000 },
    @{ n = "grupos"; a = "grupos"; e = 3000 },
    @{ n = "etiquetas"; a = "etiquetas"; e = 3000 },
    @{ n = "colecciones"; a = "colecciones"; e = 3500 },
    @{ n = "repisa"; a = "repisa" },
    @{ n = "expandir"; a = "expandir"; e = 3000 },
    @{ n = "plana"; a = "plana"; e = 3000 },
    @{ n = "buscar"; a = "buscar"; e = 8000 },
    @{ n = "renombrar-inline"; a = "renombrar-inline" },
    @{ n = "nueva-carpeta"; a = "nueva-carpeta"; e = 3000 },
    @{ n = "descarga"; a = "descarga" },
    @{ n = "vistarapida"; a = "vistarapida"; e = 3000 },
    @{ n = "markdown"; a = "markdown"; e = 3000 },
    @{ n = "markdown-vistarapida"; a = "markdown,vistarapida"; e = 3000 },
    @{ n = "pdf"; a = "pdf"; e = 4000 },
    @{ n = "pdf-vistarapida"; a = "pdf,vistarapida"; e = 4000 },
    @{ n = "red"; a = "red"; e = 5000 },
    @{ n = "toast"; a = "toast"; e = 1600 },
    @{ n = "menu"; a = "menu"; e = 3000 },
    @{ n = "menu-barra"; a = "menu-barra"; e = 2500 },
    @{ n = "menu-favoritos"; a = "menu-favoritos"; e = 2500 },
    @{ n = "historial"; a = "historial" },
    @{ n = "sugerencias"; a = "sugerencias"; e = 2800 },
    @{ n = "migas"; a = "migas" }
)
$filtro = @($solo -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$temas = if ($tema) { @($tema) } else { @("oscuro", "claro") }

$ok = @(); $mal = @()
$reloj = [Diagnostics.Stopwatch]::StartNew()
foreach ($c in $capturas) {
    if ($filtro.Count -and $filtro -notcontains $c.n) { continue }
    foreach ($t in $temas) {
        $png = Join-Path $salida "$($c.n)-$t.png"
        $argv = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "$PSScriptRoot\captura.ps1", "-exe", $exe, "-out", $png)
        if ($c.a) { $argv += @("-abrir", $c.a) }
        if ($c.v) { $argv += @("-vista", $c.v) }
        # Por defecto 3 s: da tiempo a que se cargue la miniatura de la vista previa.
        $argv += @("-espera", [string]$(if ($c.e) { $c.e } else { 3000 }))
        if ($t -eq "claro") { $argv += "-claro" }
        Write-Host -NoNewline ("{0,-24} {1,-7} " -f $c.n, $t)
        $salidaTexto = & powershell.exe @argv 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0 -and (Test-Path $png)) {
            Write-Host "ok"
            $ok += "$($c.n)-$t"
        } else {
            $motivo = ($salidaTexto -split "`r?`n" | Where-Object { $_ -match "\S" -and $_ -notmatch "^\s*(\+|En |At |CategoryInfo|FullyQualifiedErrorId)" } | Select-Object -First 1)
            Write-Host "FALLÓ: $motivo" -ForegroundColor Red
            $mal += "$($c.n)-$t ($motivo)"
        }
    }
}

""
"Capturas en $salida  ({0:N0} s)" -f $reloj.Elapsed.TotalSeconds
"  bien:  $($ok.Count)"
"  fallaron: $($mal.Count)"
foreach ($m in $mal) { "    - $m" }
if ($mal.Count) { exit 1 }
