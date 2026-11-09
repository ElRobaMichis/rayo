# Prueba de humo antes de publicar: abre el rayo.exe compilado en una instancia aislada (sus
# propios datos, sin tocar la configuración real) y comprueba que:
#   - la ventana aparece pronto,
#   - responde durante unos segundos,
#   - no quedan errores en errores.log,
#   - una configuración de una versión anterior se sigue leyendo.
# Uso: pwsh scripts/smoke.ps1 [ruta de rayo.exe]
param([string]$exe = "$PSScriptRoot\..\target\release\rayo.exe")

$ErrorActionPreference = "Stop"
$exe = (Resolve-Path $exe).Path
$data = Join-Path $env:TEMP "rayo-humo-$PID"
$folder = Join-Path $data "carpeta"
New-Item -ItemType Directory -Force -Path $folder | Out-Null
1..50 | ForEach-Object { Set-Content -Path (Join-Path $folder "archivo-$_.txt") -Value "x" }
# Configuración de 0.2.4, con las pestañas apuntando a la carpeta de prueba.
$cfg = Get-Content "$PSScriptRoot\..\src\ui\testdata\config-0.2.4.json" -Raw | ConvertFrom-Json
$cfg.tabs = @($folder); $cfg.tabs_right = @($folder); $cfg.active_tab = 0
$cfg.start_with_windows = $false; $cfg.default_explorer = $false
$cfg | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $data "config.json") -Encoding utf8

$env:RAYO_DATA_DIR = $data
$env:RAYO_NO_ACTIVATE = "1"   # sin quitarle el foco a lo que se esté usando
$fail = @()
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process $exe -ArgumentList "`"$folder`"" -PassThru
while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 10000) { Start-Sleep -Milliseconds 50; $p.Refresh() }
$shown = $sw.ElapsedMilliseconds
if ($p.MainWindowHandle -eq 0) { $fail += "la ventana no apareció en 10 s" }
foreach ($i in 1..6) {
    Start-Sleep -Milliseconds 500
    $p.Refresh()
    if ($p.HasExited) { $fail += "se cerró sola (código $($p.ExitCode))"; break }
    if (-not $p.Responding) { $fail += "no responde a los $($i * 0.5) s" }
}
$ram = if (-not $p.HasExited) { [math]::Round($p.WorkingSet64 / 1MB) } else { 0 }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
$log = Join-Path $data "errores.log"
if ((Test-Path $log) -and (Get-Item $log).Length -gt 0) { $fail += "errores.log: " + (Get-Content $log -Raw) }
$cfgBack = Join-Path $data "config.json.bak"
if (Test-Path $cfgBack) { $fail += "la configuración de 0.2.4 no se leyó entera (ver diagnostico.log)" }

"Ventana en $shown ms · RAM $ram MB"
Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
if ($fail.Count -gt 0) { $fail | ForEach-Object { Write-Host "FALLA: $_" -ForegroundColor Red }; exit 1 }
Write-Host "Prueba de humo: bien" -ForegroundColor Green
