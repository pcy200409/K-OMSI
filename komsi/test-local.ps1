<#
 K-OMSI: the full workspace test on this PC, without the tests that open network ports.

   .\komsi\test-local.ps1            run it (cargo test --locked --no-fail-fast)
   .\komsi\test-local.ps1 -Watch     also report any test process listening on a non-loopback
                                     address (that is what makes Windows Firewall ask "allow
                                     this app?" once per freshly built test exe)

 The left-out tests (LAN hosting, admin panel, remote control, host ports) bind 0.0.0.0, so
 every new build's test exes would raise a firewall prompt each. CI's windows-tests job in
 release.yml still runs them all before a release is published.
#>
param([switch]$Watch)
Set-Location (git rev-parse --show-toplevel)

# whole crates whose tests host on all addresses
$skipCrates = @('omsi-admin', 'omsi-net', 'omsi-lan', 'omsi-remote')
# omsi-app (openomsi_game): only its modules that host a LAN session or a server
$skipAppTests = @('app_impl::rooms::', 'app_impl::room_world::', 'app_impl::shell_link::', 'server::',
    'shell::pause::online_ui::', 'remote::', 'remote_panel::', 'game_panels::multiplayer::')

$watcher = $null
if ($Watch) {
    $watcher = Start-Job {
        $seen = @{}
        while ($true) {
            $tcp = Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalAddress -notin '127.0.0.1', '::1' }
            $udp = Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object { $_.LocalAddress -notin '127.0.0.1', '::1' }
            foreach ($e in @($tcp) + @($udp)) {
                if (-not $e) { continue }
                $p = Get-Process -Id $e.OwningProcess -ErrorAction SilentlyContinue
                if ($p -and $p.Path -match '\\target\\') {
                    $k = "$($p.ProcessName) $($e.LocalAddress):$($e.LocalPort)"
                    if (-not $seen[$k]) { $seen[$k] = 1; $k }
                }
            }
            Start-Sleep -Milliseconds 200
        }
    }
}

$ex = $skipCrates | ForEach-Object { '--exclude', $_ }
cargo test --locked --workspace --no-fail-fast @ex --exclude omsi-app
$a = $LASTEXITCODE
$sk = $skipAppTests | ForEach-Object { '--skip', $_ }
cargo test --locked --no-fail-fast -p omsi-app -- @sk
$b = $LASTEXITCODE

if ($watcher) {
    Stop-Job $watcher
    $hits = Receive-Job $watcher
    Remove-Job $watcher
    if ($hits) { Write-Host "`nTest processes listening on all addresses (firewall prompts):" -ForegroundColor Yellow; $hits }
    else { Write-Host "`nNo test process listened on a non-loopback address." -ForegroundColor Green }
}
Write-Host "Left out here (CI runs them): crates $($skipCrates -join ', '); omsi-app tests matching $($skipAppTests -join ' ')"
if ($a -ne 0 -or $b -ne 0) { exit 1 }
