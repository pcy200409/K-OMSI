<#
 K-OMSI: let ReShade load into the game (the release build's step, see .github/workflows/release.yml).

   pwsh komsi/reshade-workaround.ps1 -Target x86_64-pc-windows-msvc           before the build
   pwsh komsi/reshade-workaround.ps1 -Target x86_64-pc-windows-msvc -Verify   after it

 wgpu makes IDXGIFactoryMedia through the legacy CreateDXGIFactory1 (wgpu-hal,
 src/dx12/instance.rs). ReShade hooks that legacy export, and the process then dies with an
 access violation inside dxgi.dll before any window shows. openOMSI never makes a composition
 swap chain, the only thing that factory is for, and wgpu treats it as optional (`.ok()`), so the
 call is taken out. After Criisko/openOMSI's workaround (MIT).

 The workspace patches wgpu-hal to the org's wgpu fork (git), whose crates depend on each other by
 path inside that repository: a copy beside it would be a second wgpu-hal. So the line is taken
 out of the checkout cargo itself fetched, on the build machine only - meant for CI. (On a
 developer's PC that checkout is shared by every build; there a patched copy of the fork and
 `cargo build --config` with [patch.crates-io] paths does the same without touching it.)

 A git dependency is fingerprinted by its revision, not its files, so wgpu-hal is cleaned and
 compiled again; -Verify then checks that the built program no longer asks for the factory.
#>
param([Parameter(Mandatory)][string]$Target, [switch]$Verify)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

if ($Verify) {
    $exe = "target/$Target/release/openomsi.exe"
    $bytes = [IO.File]::ReadAllBytes($exe)
    $latin1 = [Text.Encoding]::GetEncoding(28591)
    # IDXGIFactoryMedia's interface id, as the removed call carries it
    $needle = $latin1.GetString([byte[]](0xF2, 0xD1, 0xE7, 0x41, 0x91, 0xA5, 0x7B, 0x4F, 0xA2, 0xE5, 0xFA, 0x9C, 0x84, 0x3E, 0x1C, 0x12))
    if ($latin1.GetString($bytes).Contains($needle)) {
        throw "the ReShade workaround is not in $exe (wgpu-hal was not compiled again)"
    }
    Write-Host "$exe no longer asks for IDXGIFactoryMedia"
    exit 0
}

$toml = Get-Content Cargo.toml -Raw
if ($toml -notmatch '(?m)^wgpu-hal = \{ git = "([^"]+)", rev = "([^"]+)" \}') {
    throw 'wgpu-hal is not patched to a git revision in Cargo.toml any more: look at the ReShade workaround'
}
$short = $Matches[2].Substring(0, 7)
cargo fetch --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo fetch failed' }
$home_ = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$file = Get-ChildItem (Join-Path $home_ 'git\checkouts') -Directory |
    Where-Object { $_.Name -like 'wgpu-openomsi-*' } |
    ForEach-Object { Join-Path $_.FullName "$short\wgpu-hal\src\dx12\instance.rs" } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $file) { throw "the wgpu fork at $short is not in cargo's checkouts after cargo fetch" }

$text = Get-Content $file -Raw
if ($text -match 'K-OMSI: IDXGIFactoryMedia') {
    Write-Host "already patched: $file"
} else {
    $note = @(
        '        // (K-OMSI: IDXGIFactoryMedia is made through the legacy CreateDXGIFactory1, which',
        '        // ReShade hooks - the process then dies in dxgi.dll before any window shows. It is',
        '        // only used for composition swap chains, which openOMSI never makes.)',
        '        let factory_media: Option<Dxgi::IDXGIFactoryMedia> = None;',
        '        let _ = &lib_dxgi;'
    ) -join "`n"
    $pattern = '(?m)^[ \t]*// Create IDXGIFactoryMedia\r?\n[ \t]*let factory_media = lib_dxgi\.create_factory_media\(\)\.ok\(\);'
    $patched = $text -replace $pattern, $note
    if ($patched -eq $text) { throw "the line is not in $file any more: look at the ReShade workaround" }
    Set-Content $file $patched -NoNewline
    Write-Host "patched $file"
}
# (the cache may hold a wgpu-hal built from the unpatched source)
cargo clean --release --target $Target -p wgpu-hal
if ($LASTEXITCODE -ne 0) { throw 'cargo clean -p wgpu-hal failed' }
