# Captura de la ventana de Rayo para revisar el diseño: abre el rayo.exe compilado en una
# instancia aislada (sus propios datos, sin tocar la configuración real), sobre una carpeta de
# ejemplo, y guarda una imagen de la ventana (la saca Rayo mismo: ver src/ui/capture.rs).
# Uso: powershell -File scripts/captura.ps1 -out captura.png [-claro] [-vista Details|Columns|Grid] [-abrir paso1,paso2] [-exe ruta]
# Los pasos de -abrir (hash, renombrar, terminal, git, cuatro…) están en src/ui/capture_steps.rs;
# algunos necesitan algo en la carpeta de ejemplo y eso se prepara aquí (git, copias, pdf…).
param(
    [string]$out = "$PSScriptRoot\..\target\captura.png",
    [switch]$claro,
    [string]$vista = "Details",
    [string]$exe = "$PSScriptRoot\..\target\release\rayo.exe",
    [int]$ancho = 1280,
    [int]$alto = 760,
    [int]$espera = 2500,
    [string]$abrir = "",
    # Abrir una carpeta real en vez de la de ejemplo (p. ej. el escritorio).
    [string]$carpeta = "",
    # Carpeta de red para el paso `red`.
    [string]$red = "\\localhost\C$\Windows"
)

$ErrorActionPreference = "Stop"
# Texto en UTF-8 sin BOM (Set-Content de Windows PowerShell escribe en ANSI y Rayo leería «Jap?n»).
$utf8 = New-Object Text.UTF8Encoding $false
function Escribe($path, $text) { [IO.File]::WriteAllText($path, $text, $utf8) }
function Agrega($path, $text) { [IO.File]::AppendAllText($path, $text, $utf8) }
$exe = (Resolve-Path $exe).Path
$pasos = @($abrir -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ })
function Pide($p) { return $pasos -contains $p }

$data = Join-Path $env:TEMP "rayo-captura-$PID"
$base = Join-Path $data "Imágenes"
$folder = Join-Path $base "Japón 2026"
New-Item -ItemType Directory -Force -Path $folder | Out-Null
foreach ($d in "Kioto", "Osaka", "Hakone") {
    $sub = Join-Path $folder $d
    New-Item -ItemType Directory -Force -Path $sub | Out-Null
    1..6 | ForEach-Object { Set-Content -Path (Join-Path $sub "foto-$_.txt") -Value ("x" * (4000 * $_)) }
}
$foto = Join-Path $PSScriptRoot "..\src\fsx\testdata\foto.jpg"
Copy-Item $foto (Join-Path $folder "torii-fushimi.jpg")
Copy-Item $foto (Join-Path $folder "arashiyama-bambu.jpg")
Escribe (Join-Path $folder "itinerario.md") "# Japón 2026`n`n- Día 1: Tokio`n- Día 3: Hakone`n- Día 4: Kioto`n"
Escribe (Join-Path $folder "presupuesto.csv") "concepto,monto`nvuelo,1200`nhoteles,1400`n"
Set-Content -Path (Join-Path $folder "reservas-hoteles.pdf") -Value ("%PDF-1.4" + ("x" * 300000))
Escribe (Join-Path $folder "notas.txt") "Llevar adaptador de enchufe tipo A.`n"

# ---- lo que necesita cada paso
if (Pide "markdown") {
    # Un markdown con un poco de todo para ver cómo se dibuja.
    $md = "# Japón 2026`n`nDoce noches entre **Tokio**, *Hakone* y Kioto.`n`n## Itinerario`n`n" +
        "1. Día 1: llegada a Tokio, Shibuya de noche`n2. Día 3: Hakone, ryokan con onsen`n3. Día 4: Kioto, Fushimi Inari al amanecer`n`n" +
        "## Pendientes`n`n- [x] Pasaportes`n- [ ] JR Pass (``7 días``)`n- [ ] Adaptador tipo A`n`n" +
        "> Reservar el tren bala con tiempo en temporada de sakura.`n`n" +
        "| Ciudad | Noches | Hotel |`n|---|---|---|`n| Tokio | 4 | Shinjuku |`n| Hakone | 2 | Ryokan |`n| Kioto | 6 | Gion |`n"
    Escribe (Join-Path $folder "itinerario.md") $md
}
if (Pide "pdf") {
    # Un PDF de verdad (una página), para que el lector de Windows lo dibuje.
    $lines = @(
        "0.16 0.38 0.85 rg 0 752 595 90 re f",
        "1 1 1 rg BT /F1 28 Tf 50 795 Td (Reservas de hoteles) Tj ET",
        "1 1 1 rg BT /F2 14 Tf 50 772 Td (Japón 2026 - 12 noches, 3 ciudades) Tj ET",
        "0.2 0.2 0.2 rg BT /F1 13 Tf 50 700 Td (Ciudad) Tj 140 0 Td (Hotel) Tj 200 0 Td (Noches) Tj 90 0 Td (Total) Tj ET",
        "0.8 G 1 w 50 690 m 545 690 l S",
        "0.1 0.1 0.1 rg BT /F2 12 Tf 50 670 Td (Tokio) Tj 140 0 Td (Hotel Shinjuku) Tj 200 0 Td (4) Tj 90 0 Td (USD 620) Tj ET",
        "BT /F2 12 Tf 50 646 Td (Hakone) Tj 140 0 Td (Ryokan Fuji-ya) Tj 200 0 Td (2) Tj 90 0 Td (USD 480) Tj ET",
        "BT /F2 12 Tf 50 622 Td (Kioto) Tj 140 0 Td (Gion Machiya) Tj 200 0 Td (6) Tj 90 0 Td (USD 300) Tj ET",
        "0.8 G 50 606 m 545 606 l S",
        "BT /F1 12 Tf 50 586 Td (Total) Tj 430 0 Td (USD 1400) Tj ET",
        "0.95 0.95 0.95 rg 50 420 495 140 re f",
        "0.3 0.3 0.3 rg BT /F2 11 Tf 64 540 Td (Check-in desde las 15:00. Cancelacion gratuita hasta 7 dias antes.) Tj ET",
        "BT /F2 11 Tf 64 522 Td (Presentar el pasaporte al llegar. Desayuno incluido en Hakone.) Tj ET"
    )
    $content = ($lines -join "`n") + "`n"
    $enc = [Text.Encoding]::GetEncoding(1252)
    $objs = @(
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        "<< /Length $($enc.GetByteCount($content)) >>`nstream`n$content" + "endstream"
    )
    $pdf = "%PDF-1.4`n"
    $offs = @()
    for ($i = 0; $i -lt $objs.Count; $i++) {
        $offs += $enc.GetByteCount($pdf)
        $pdf += "$($i + 1) 0 obj`n$($objs[$i])`nendobj`n"
    }
    $xref = $enc.GetByteCount($pdf)
    $pdf += "xref`n0 $($objs.Count + 1)`n0000000000 65535 f `n"
    foreach ($o in $offs) { $pdf += ('{0:D10} 00000 n ' -f $o) + "`n" }
    $pdf += "trailer`n<< /Size $($objs.Count + 1) /Root 1 0 R >>`nstartxref`n$xref`n%%EOF`n"
    [IO.File]::WriteAllBytes((Join-Path $folder "reservas-hoteles.pdf"), $enc.GetBytes($pdf))
}
if (Pide "descarga") {
    # Una descarga de Chrome a medias.
    Set-Content -Path (Join-Path $folder "mapa-kioto.pdf.crdownload") -Value ("x" * 900000)
}
if (Pide "copias") {
    # Un archivo grande (se crea al instante: NTFS no escribe los ceros) para que la copia siga
    # en curso al capturar.
    $fs = [IO.File]::Create((Join-Path $folder "video-hakone.mp4"))
    $fs.SetLength(16GB)
    $fs.Close()
}
if (Pide "comparar") {
    # Kioto y Osaka casi iguales: uno con un archivo de más, otro con uno distinto.
    Escribe (Join-Path $folder "Osaka\dotonbori.txt") "Takoyaki y luces de neón.`n"
    Escribe (Join-Path $folder "Kioto\templos.txt") "Kinkaku-ji, Ginkaku-ji, Kiyomizu-dera.`n"
    Set-Content -Path (Join-Path $folder "Osaka\foto-3.txt") -Value ("y" * 9000)
}
(Get-Item (Join-Path $folder "reservas-hoteles.pdf")).LastWriteTime = (Get-Date).AddDays(-60)
(Get-Item (Join-Path $folder "presupuesto.csv")).LastWriteTime = (Get-Date).AddDays(-75)
(Get-Item (Join-Path $folder "Hakone")).LastWriteTime = (Get-Date).AddDays(-20)
(Get-Item (Join-Path $folder "Osaka")).LastWriteTime = (Get-Date).AddDays(-7)
if (Pide "git") {
    # Un repositorio con cambios: uno modificado, uno nuevo agregado y uno sin seguir.
    Push-Location $folder
    try {
        $g = @("-c", "user.name=Rayo", "-c", "user.email=rayo@example.com", "-c", "core.autocrlf=false", "-c", "init.defaultBranch=main")
        & git @g init -q 2>&1 | Out-Null
        & git @g add -A 2>&1 | Out-Null
        & git @g commit -q -m "Viaje a Japón" 2>&1 | Out-Null
        Agrega (Join-Path $folder "itinerario.md") "- Día 8: Nara`n"
        Agrega (Join-Path $folder "presupuesto.csv") "trenes,350`n"
        Escribe (Join-Path $folder "Osaka\comida.txt") "Okonomiyaki en Dotonbori.`n"
        & git @g add "Osaka/comida.txt" 2>&1 | Out-Null
        Escribe (Join-Path $folder "pendientes.txt") "Comprar JR Pass.`n"
        Remove-Item (Join-Path $folder "Hakone\foto-6.txt")
    } finally { Pop-Location }
}
$lock = $null
if (Pide "bloqueos") {
    # notas.txt queda abierto (sin compartir) por este PowerShell mientras Rayo está abierto.
    $lock = [IO.File]::Open((Join-Path $folder "notas.txt"), 'Open', 'ReadWrite', 'None')
}
if (Pide "red") {
    if (-not (Test-Path -LiteralPath $red)) {
        Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
        Write-Error "la carpeta de red $red no está accesible"
    }
    $env:RAYO_CAPTURA_RED = $red
}

$cfg = Get-Content "$PSScriptRoot\..\src\ui\testdata\config-0.2.4.json" -Raw | ConvertFrom-Json
# Asignar aunque la configuración de prueba no traiga esa propiedad.
function Pon($name, $value) { $cfg | Add-Member -NotePropertyName $name -NotePropertyValue $value -Force }
Pon tabs @($base); Pon tabs_right @($folder); Pon active_tab 0; Pon split $false
Pon dark (-not $claro); Pon show_preview $true; Pon view $vista; Pon sort "Name"; Pon sort_desc $false
Pon default_view @{ view = $vista; sort = "Name"; desc = $false }; Pon folder_views @()
Pon start_with_windows $false; Pon default_explorer $false; Pon places_collapsed $false; Pon lang "es"
# Sin el diálogo de primera vez (aunque este Rayo no esté instalado): eso lo muestra el paso `primer-uso`.
Pon portable $true
Pon groups @(@{ name = "Favoritos"; items = @($folder, $base); collapsed = $false })
if ($carpeta) { Pon tabs @($carpeta) }
$cfg | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $data "config.json") -Encoding utf8

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Cap {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
}
"@

$env:RAYO_DATA_DIR = $data
$env:RAYO_NO_ACTIVATE = "1"
$out = [IO.Path]::GetFullPath($out)
Remove-Item $out -ErrorAction SilentlyContinue
$env:RAYO_CAPTURA = $out
$env:RAYO_CAPTURA_ESPERA = [string]($espera / 1000)
$env:RAYO_CAPTURA_ABRIR = $abrir
$env:RAYO_CAPTURA_CARPETA = $folder
$arg = if ($carpeta) { $carpeta } else { Join-Path $folder 'torii-fushimi.jpg' }
try {
    $p = Start-Process $exe -ArgumentList "`"$arg`"" -PassThru
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 15000) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    if ($p.MainWindowHandle -eq 0) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Write-Error "la ventana no apareció" }
    # SWP_NOZORDER | SWP_NOACTIVATE
    [Cap]::SetWindowPos($p.MainWindowHandle, [IntPtr]::Zero, 40, 40, $ancho, $alto, 0x14) | Out-Null
    # Rayo guarda la captura solo (ver src/ui/capture.rs) y se cierra.
    if (-not $p.WaitForExit($espera + 10000)) { Stop-Process -Id $p.Id -Force; Write-Error "no se cerró: sin captura" }
} finally {
    if ($lock) { $lock.Close() }
    # La terminal integrada (paso `terminal`) deja su consola: que no quede viva.
    Get-CimInstance Win32_Process -Filter "Name='pwsh.exe' or Name='powershell.exe' or Name='cmd.exe' or Name='git.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ParentProcessId -eq $p.Id } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
}
if (-not (Test-Path $out)) { Write-Error "Rayo se cerró sin guardar la captura" }
"Captura: $out"
