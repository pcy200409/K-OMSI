<#
 Bring K-OMSI up to date with openOMSI and re-apply our patches.

   .\komsi\sync-upstream.ps1            fetch upstream, rebase every patch branch onto it, rebuild `release`
   .\komsi\sync-upstream.ps1 -Check     only report what is new upstream and which of our files it touches
   .\komsi\sync-upstream.ps1 -Build     also run the omsi-sim tests and cargo check afterwards
   .\komsi\sync-upstream.ps1 -Continue  after you fixed a rebase conflict (git add; git rebase --continue)

 Branches: main = openOMSI exactly (never edited) | branding, patch/* = our changes on top of main
           release = main + branding + patches merged, the branch K-OMSI is built from.
 A patch upstream has already taken in is dropped by the rebase on its own.
 Nothing is pushed; the final lines say what to push.
#>
param([switch]$Check, [switch]$Build, [switch]$Continue)
$ErrorActionPreference = 'Stop'
Set-Location (git rev-parse --show-toplevel)
$id = @('-c','user.name=pcy200409','-c','user.email=154576452+pcy200409@users.noreply.github.com')
$branches = Get-Content komsi/patches.txt | Where-Object { $_ -and $_ -notmatch '^\s*#' } | ForEach-Object { $_.Trim() }
function Stop-Msg($m) { Write-Host $m -ForegroundColor Red; exit 1 }

if (-not $Continue) {
    if (git status --porcelain) { Stop-Msg 'Working tree has uncommitted changes. Commit or set them aside first.' }
    git fetch upstream --tags --prune
    $old = git rev-parse main
    $new = git rev-parse upstream/main
    if ($old -eq $new) { Write-Host 'openOMSI has nothing new.' -ForegroundColor Green; if (-not $Check) { Write-Host 'Rebuilding release anyway.' } }
    else {
        $n = git rev-list --count "$old..$new"
        Write-Host "openOMSI is $n commits ahead:" -ForegroundColor Cyan
        git log --oneline --no-merges "$old..$new" | Select-Object -First 25
        $upFiles = git diff --name-only $old $new
        foreach ($b in $branches) {
            $mine = git diff --name-only $old $b
            $both = $mine | Where-Object { $upFiles -contains $_ }
            if ($both) { Write-Host "`n$b touches files openOMSI changed too (conflict possible):" -ForegroundColor Yellow; $both | ForEach-Object { "  $_" } }
        }
        git tag --points-at $new | ForEach-Object { Write-Host "New release tag: $_" -ForegroundColor Cyan }
    }
    if ($Check) { exit 0 }
    git checkout -q main
    git reset -q --hard upstream/main
    foreach ($b in $branches) {
        Write-Host "`nRebasing $b onto main ..." -ForegroundColor Cyan
        git @id rebase main $b
        if ($LASTEXITCODE -ne 0) {
            Write-Host "`nConflict in $b. Files:" -ForegroundColor Yellow
            git diff --name-only --diff-filter=U
            Stop-Msg "Fix them, then: git add <files>; git rebase --continue; .\komsi\sync-upstream.ps1 -Continue"
        }
    }
}
else {
    if (Test-Path "$(git rev-parse --git-dir)/rebase-merge") { Stop-Msg 'A rebase is still in progress. git rebase --continue first.' }
    # patches already rebased stay as they are; rebase the rest onto main
    foreach ($b in $branches) {
        if ((git merge-base --is-ancestor main $b; $LASTEXITCODE) -ne 0) {
            Write-Host "Rebasing $b onto main ..." -ForegroundColor Cyan
            git @id rebase main $b
            if ($LASTEXITCODE -ne 0) { git diff --name-only --diff-filter=U; Stop-Msg "Conflict in $b. Fix, git add, git rebase --continue, rerun -Continue." }
        }
    }
}

git checkout -q -B release main
foreach ($b in $branches) { git @id merge -q --no-ff -m "K-OMSI: merge $b" $b; if ($LASTEXITCODE -ne 0) { Stop-Msg "Merging $b into release failed." } }
Write-Host "`nrelease rebuilt: main + $($branches -join ' + ')" -ForegroundColor Green

if ($Build) {
    cargo test -p omsi-sim --lib
    if ($LASTEXITCODE -ne 0) { Stop-Msg 'omsi-sim tests failed.' }
    cargo check -p omsi-app
    if ($LASTEXITCODE -ne 0) { Stop-Msg 'cargo check failed.' }
    Write-Host 'Build and tests passed.' -ForegroundColor Green
}
Write-Host "`nTo publish (rebased branches need --force-with-lease):"
Write-Host "  git push --force-with-lease origin main $($branches -join ' ') release"
