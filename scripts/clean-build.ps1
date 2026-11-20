# Keep the Rust build directories bounded: sweep artifacts older than a
# week, drop the coverage build, and remove a directory whole when it is
# still above the cap (a full rebuild is minutes; 100 GB of stale objects
# was the alternative). See docs/DEVELOPMENT.md "Build cache".
#
#   ./scripts/clean-build.ps1            # sweep, cap 15 GB
#   ./scripts/clean-build.ps1 -CapGB 5   # tighter cap
#   ./scripts/clean-build.ps1 -All       # remove every build directory now
param(
    [int]$CapGB = 15,
    [int]$Days = 7,
    [switch]$All
)
$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$dirs = @(
    (Join-Path $repo 'target'),
    (Join-Path (Split-Path -Parent $repo) 'target-claude')
)
if ($env:CARGO_TARGET_DIR) { $dirs += $env:CARGO_TARGET_DIR }
$dirs = $dirs | Select-Object -Unique | Where-Object { Test-Path $_ }

function Size-GB($path) {
    $bytes = (Get-ChildItem -LiteralPath $path -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
    if (-not $bytes) { $bytes = 0 }
    [math]::Round($bytes / 1GB, 1)
}

foreach ($d in $dirs) {
    $before = Size-GB $d
    if ($All) {
        Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ("{0}: {1} GB -> removed" -f $d, $before)
        continue
    }
    # The instrumented coverage build is rebuilt on the next measurement anyway.
    $cov = Join-Path $d 'llvm-cov-target'
    if (Test-Path $cov) { Remove-Item -LiteralPath $cov -Recurse -Force -ErrorAction SilentlyContinue }
    if (Get-Command cargo-sweep -ErrorAction SilentlyContinue) {
        & cargo sweep --time $Days --path $repo --target $d 2>$null | Out-Null
    } else {
        Write-Host "cargo-sweep not installed (cargo install cargo-sweep): only the cap applies"
    }
    $after = Size-GB $d
    if ($after -gt $CapGB) {
        Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ("{0}: {1} GB -> {2} GB after sweep, above {3} GB: removed" -f $d, $before, $after, $CapGB)
    } else {
        Write-Host ("{0}: {1} GB -> {2} GB" -f $d, $before, $after)
    }
}
$web = Join-Path $repo 'web\coverage'
if (Test-Path $web) { Remove-Item -LiteralPath $web -Recurse -Force -ErrorAction SilentlyContinue }

# The tests leave their stores in the temp directory; the npm cache is unbounded.
$tmp = [System.IO.Path]::GetTempPath()
Get-ChildItem -LiteralPath $tmp -Directory -Filter 'ontology-*' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-1) } |
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
if (Get-Command npm -ErrorAction SilentlyContinue) { & npm cache verify 2>$null | Out-Null }
