# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
# Copyright (C) 2026 Mediasoft & Cie S.A.
#
# (Re)importe le modele logiciel MS.ERP dans un store, dans l'ordre :
#   1. (re)construit le binaire `ontology` en release,
#   2. `--xref --sync`  : la reference croisee du compilateur (programmes, tables, champs, liens),
#   3. `--abl --sync`   : les sources ABL (jointures, ecrans, onglets),
#   4. `--df`           : les dumps de schema (.df) de chaque base — libelles, types, cles.
# Idempotent : relancer apres une recompilation ou un nouveau .df met a jour ce qui a change.
#
# Exemples :
#   ./scripts/import-erp.ps1                      # store .\data, chemins par defaut
#   ./scripts/import-erp.ps1 -DataDir .\data-test
#   ./scripts/import-erp.ps1 -SkipXref -SkipAbl   # seulement les .df

[CmdletBinding()]
param(
    [string]$DataDir = ".\data",
    [string]$ErpRoot = "$env:USERPROFILE\Desktop\MS.ERP",
    [string]$DfDir = "$env:USERPROFILE\Desktop\ai-ontology\erp-data",
    # Racines de compilation retirees des chemins des xref.
    [string[]]$XrefRoots = @('D:\Mediasoft\App\MS.ERP\', 'D:\Mediasoft\App\MS.ERP2\'),
    # Fichier .df -> nom de la base qu'il decrit.
    [hashtable]$Dumps = @{ 'msdb201026.df' = 'msdb'; 'msecdefinition.df' = 'msec'; 'testwkdb.df' = 'wkdb' },
    [switch]$SkipXref,
    [switch]$SkipAbl,
    [switch]$SkipDf,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# Un serveur qui tient le store (`npm run dev`, `ontology serve`) fait echouer
# la premiere etape avec « store is locked by another process » : l'arreter d'abord.

if (-not $SkipBuild) {
    cargo build --release -p ontology-cli --manifest-path (Join-Path $repo 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw "cargo build a echoue" }
}
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$bin = Join-Path $target 'release\ontology.exe'

function Step([string]$label, [string[]]$argv) {
    Write-Host "== $label" -ForegroundColor Cyan
    $t = [Diagnostics.Stopwatch]::StartNew()
    & $bin ingest --data $DataDir @argv
    if ($LASTEXITCODE -ne 0) { throw "$label a echoue (exit $LASTEXITCODE)" }
    Write-Host ("   {0:n0} s" -f $t.Elapsed.TotalSeconds)
}

if (-not $SkipXref) {
    $roots = $XrefRoots | ForEach-Object { '--xref-root', $_ }
    Step 'xref' (@('--xref', '--sync') + $roots + (Join-Path $ErpRoot 'MS.ERP_xref_compiled'))
}
if (-not $SkipAbl) {
    Step 'abl' @('--abl', '--sync', (Join-Path $ErpRoot 'MS.ERP_sources\MS.ERP2'))
}
if (-not $SkipDf) {
    foreach ($file in $Dumps.Keys | Sort-Object) {
        Step "df $($Dumps[$file])" @('--df', '--df-db', $Dumps[$file], (Join-Path $DfDir $file))
    }
}
Write-Host "Termine." -ForegroundColor Green
