# K-OMSI OBIS edition: puts the edition's plugins into a built game folder.
#
#   set KOMSI_EDITION=obis & scripts\build-windows.cmd      (the game with OBIS: --features omsi-app/obis)
#   powershell -File komsi\obis-edition\package.ps1 -Dist dist\windows -ObisClient "<OMSI 2>\OBIS-client"
#
# Adds Plugins\komsi_obis (OBIS panel, route strip, sending the player's bus) and Plugins\komsi_taco
# (in-game tachograph and recording; the scoring stays in the outside TACO analyser).
# The OBIS route data is not in this repository: it is OBIS's, taken from the OBIS client installed on
# the packaging computer (data\routes -> komsi_obis\obis_routes, by build_obis_routes.py).
# -Names: a folder with stop_names_ko.json / obis_names_ko.json (Korean stop names), optional.
# The plain edition is built without KOMSI_EDITION and gets none of this.
param(
    [string]$Dist = "dist\windows",
    [Parameter(Mandatory = $true)][string]$ObisClient,
    [string]$Names = ""
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not (Test-Path (Join-Path $ObisClient "data\routes"))) { throw "no data\routes in $ObisClient" }
$plugins = Join-Path $Dist "Plugins"
foreach ($p in "komsi_obis", "komsi_taco") {
    $to = Join-Path $plugins $p
    if (Test-Path $to) { Remove-Item -Recurse -Force $to }
    Copy-Item -Recurse (Join-Path $here "plugins\$p") $to
}
python (Join-Path $here "build_obis_routes.py") $ObisClient (Join-Path $plugins "komsi_obis")
if ($LASTEXITCODE -ne 0) { throw "build_obis_routes.py failed" }
if ($Names) {
    foreach ($f in "stop_names_ko.json", "obis_names_ko.json") {
        $src = Join-Path $Names $f
        if (Test-Path $src) { Copy-Item $src (Join-Path $plugins "komsi_obis\$f") }
    }
}
Write-Host "OBIS edition plugins in $plugins"
