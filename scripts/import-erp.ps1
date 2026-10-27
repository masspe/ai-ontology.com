# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
# Copyright (C) 2026 Mediasoft & Cie S.A.
#
# (Re)importe un logiciel Progress OpenEdge comme modele dans un store, dans l'ordre :
#   1. (re)construit le binaire `ontology` en release,
#   2. `--xref --sync`  : la reference croisee du compilateur (programmes, tables, champs, liens),
#   3. `--abl --sync`   : les sources ABL (jointures, ecrans, onglets),
#   4. `--df`           : le dump de schema (.df) de chaque base — libelles, types, cles,
#   5. `--doc-for Screen`: l'aide utilisateur (pages HTML) attachee aux ecrans, si $HelpDir est defini,
#   6. `link-mentions`  : les pages et ecrans lies aux ecrans/programmes/tables qu'ils nomment.
# Idempotent : relancer apres une recompilation ou un nouveau .df met a jour ce qui a change.
#
# Les chemins propres a une installation ne sont pas dans ce script : ils viennent
# des parametres, ou de `scripts/import-erp.local.ps1` (non versionne, voir
# `import-erp.local.ps1.example`), qui est charge s'il existe.
#
# Exemples :
#   ./scripts/import-erp.ps1                      # tout, avec import-erp.local.ps1
#   ./scripts/import-erp.ps1 -SkipXref -SkipAbl   # seulement les .df
#   ./scripts/import-erp.ps1 -DataDir .\data-test -XrefDir D:\xref -XrefRoots 'D:\App\ERP\' `
#       -SourceDir D:\src -Dumps @{ 'D:\dumps\appdb.df' = 'appdb' }

[CmdletBinding()]
param(
    [string]$DataDir = ".\data",
    # Repertoire des `*.xref.xml` du compilateur.
    [string]$XrefDir,
    # Racines de compilation retirees des chemins des xref.
    [string[]]$XrefRoots,
    # Racine des sources ABL (.p, .w, .i).
    [string]$SourceDir,
    # Fichier .df -> nom de la base qu'il decrit.
    [hashtable]$Dumps,
    # Dossier des pages d'aide (HTML) ; vide = etape sautee.
    [string]$HelpDir,
    # Regex dont le premier groupe, dans le titre de la page ou son nom de fichier, est le code de l'ecran.
    [string]$HelpKey = '\(([a-z]{2}[0-9]{3}[a-z0-9]*)\)',
    [switch]$SkipXref,
    [switch]$SkipAbl,
    [switch]$SkipDf,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

$local = Join-Path $PSScriptRoot 'import-erp.local.ps1'
if (Test-Path $local) { . $local }
foreach ($need in @(@('XrefDir', $SkipXref), @('XrefRoots', $SkipXref), @('SourceDir', $SkipAbl), @('Dumps', $SkipDf))) {
    if (-not $need[1] -and -not (Get-Variable -Name $need[0] -ValueOnly)) {
        throw "-$($need[0]) manque : le passer en parametre ou le definir dans $local"
    }
}

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
    Step 'xref' (@('--xref', '--sync') + $roots + $XrefDir)
}
if (-not $SkipAbl) {
    Step 'abl' @('--abl', '--sync', $SourceDir)
}
if (-not $SkipDf) {
    foreach ($file in $Dumps.Keys | Sort-Object) {
        Step "df $($Dumps[$file])" @('--df', '--df-db', $Dumps[$file], $file)
    }
}
if (-not $SkipDf -and $HelpDir) {
    Step 'docs' @('--doc-for', 'Screen', '--doc-key', $HelpKey, '--doc-fallback-type', 'Document', '--doc-property', 'help', $HelpDir)
}
if (-not $SkipDf -and $HelpDir) {
    Write-Host "== mentions" -ForegroundColor Cyan
    & $bin link-mentions --data $DataDir --from Document,DocumentFragment,Screen --to Screen,Program,Table
    if ($LASTEXITCODE -ne 0) { throw "link-mentions a echoue (exit $LASTEXITCODE)" }
}
Write-Host "Termine." -ForegroundColor Green
