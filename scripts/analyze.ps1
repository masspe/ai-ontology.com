# SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
# Copyright (C) 2026 Winven AI Sarl
#
# Lance une analyse de bout en bout sur un jeu de donnees :
#   1. (re)construit le binaire `ontology` si besoin,
#   2. ingere une ontologie + des fichiers de donnees,
#   3. affiche les statistiques,
#   4. execute une serie de requetes `retrieve` et `ask`,
#   5. exporte le graphe en JSONL.
#
# Exemples :
#   ./scripts/analyze.ps1                            # jeu d'exemple « finance »
#   ./scripts/analyze.ps1 -Provider openai
#   ./scripts/analyze.ps1 -DataDir .\data-prod -Reset
#   ./scripts/analyze.ps1 -Ontology .\my-onto.json `
#                         -Inputs .\docs,.\rows.csv `
#                         -Queries 'qui a paye la facture F-001 ?'

[CmdletBinding()]
param(
    # Repertoire de stockage persistant (WAL + snapshots).
    [string]$DataDir = ".\data",

    # Fichier d'ontologie a charger en premier.
    [string]$Ontology = ".\examples\finance\ontology.json",

    # Fichiers/dossiers a ingerer apres l'ontologie. Accepte plusieurs valeurs.
    [string[]]$Inputs = @(
        ".\examples\finance\seed.jsonl",
        ".\examples\finance\relations.jsonl",
        ".\examples\finance\contracts"
    ),

    # Type de concept a appliquer aux fichiers texte (--text-type).
    [string]$TextType = "Contract",

    # Extensions des fichiers texte a ramasser dans un dossier (--text-ext).
    [string]$TextExt = "txt,md",

    # Liste de requetes lancees a la fois en `retrieve` et en `ask`.
    [string[]]$Queries = @(
        "Quels contrats Acme Labs a-t-elle signes en 2025 ?",
        "Quel est le montant total facture a Initech ?",
        "Qui a signe le contrat C-2025-002 ?"
    ),

    # Backend LLM : echo (defaut, hors-ligne), anthropic, openai, deepseek.
    [ValidateSet("echo", "anthropic", "openai", "deepseek")]
    [string]$Provider = "echo",

    # Modele a passer au backend (optionnel, defaut par provider).
    [string]$Model,

    # Top-K et profondeur d'expansion pour la retrieval.
    [int]$TopK = 8,
    [int]$Depth = 2,

    # Chemin de l'export JSONL final (`""` pour desactiver).
    [string]$ExportPath = ".\out\graph.jsonl",

    # Recompile en `release` meme si le binaire existe deja.
    [switch]$Rebuild,

    # Supprime $DataDir avant l'ingestion pour repartir d'un etat propre.
    [switch]$Reset
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

# -- Helpers -----------------------------------------------------------------

function Write-Section {
    param([string]$Title)
    Write-Host ""
    Write-Host ("=" * 72) -ForegroundColor DarkCyan
    Write-Host (" {0}" -f $Title) -ForegroundColor Cyan
    Write-Host ("=" * 72) -ForegroundColor DarkCyan
}

function Invoke-Ontology {
    param([Parameter(ValueFromRemainingArguments)] [string[]]$Arguments)
    Write-Host "> ontology $($Arguments -join ' ')" -ForegroundColor DarkGray
    & $script:OntologyExe @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "ontology a echoue (exit=$LASTEXITCODE)"
    }
}

# Repere la racine du workspace par rapport a ce script.
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# -- Build -------------------------------------------------------------------

$script:OntologyExe = Join-Path $RepoRoot "target\release\ontology.exe"
if ($Rebuild -or -not (Test-Path $script:OntologyExe)) {
    Write-Section "Build (cargo build --release -p ontology-cli)"
    cargo build --release -p ontology-cli
    if ($LASTEXITCODE -ne 0) { throw "cargo build a echoue" }
}

# -- Reset / setup -----------------------------------------------------------

if ($Reset -and (Test-Path $DataDir)) {
    Write-Section "Reset $DataDir"
    Remove-Item $DataDir -Recurse -Force
}

# -- Verification de la cle API si un provider distant est demande ----------

$envKey = switch ($Provider) {
    "anthropic" { "ANTHROPIC_API_KEY" }
    "openai"    { "OPENAI_API_KEY" }
    "deepseek"  { "DEEPSEEK_API_KEY" }
    default     { $null }
}
if ($envKey -and -not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($envKey))) {
    Write-Host "Provider=$Provider  ($envKey detecte)" -ForegroundColor Green
} elseif ($envKey) {
    throw "Variable d'environnement $envKey manquante pour --$Provider."
} else {
    Write-Host "Provider=echo (mode hors-ligne, pas d'appel reseau)" -ForegroundColor Yellow
}

$ProviderArgs = @()
if ($Provider -ne "echo") { $ProviderArgs += "--$Provider" }
if ($Model) { $ProviderArgs += "--model"; $ProviderArgs += $Model }

# -- Ingestion ---------------------------------------------------------------

Write-Section "Ingest ontologie + donnees -> $DataDir"

# 1) ontologie seule (sans donnees) : on passe un fichier JSONL vide via stdin.
if ($Ontology) {
    if (-not (Test-Path $Ontology)) { throw "Ontologie introuvable : $Ontology" }
    "" | Invoke-Ontology --data $DataDir ingest --ontology $Ontology -
}

# 2) chaque entree (fichier ou dossier) est ingeree separement, avec les
#    arguments adaptes a son extension/type.
foreach ($in in $Inputs) {
    if (-not (Test-Path $in)) {
        Write-Warning "Entree absente, ignoree : $in"
        continue
    }
    $item = Get-Item $in
    if ($item.PSIsContainer) {
        Invoke-Ontology --data $DataDir ingest --text-type $TextType --text-ext $TextExt $item.FullName
    } else {
        switch -regex ($item.Extension.ToLowerInvariant()) {
            '\.(csv)$'              { Invoke-Ontology --data $DataDir ingest --csv-type $TextType $item.FullName }
            '\.(xlsx|xls|ods)$'     { Invoke-Ontology --data $DataDir ingest --xlsx-type $TextType $item.FullName }
            '\.(jsonl|ndjson)$'     { Invoke-Ontology --data $DataDir ingest $item.FullName }
            '\.(triples|txt|md)$'   { Invoke-Ontology --data $DataDir ingest $item.FullName }
            default                 { Invoke-Ontology --data $DataDir ingest $item.FullName }
        }
    }
}

# -- Stats -------------------------------------------------------------------

Write-Section "Stats"
Invoke-Ontology --data $DataDir stats

# -- Retrieve + Ask ----------------------------------------------------------

foreach ($q in $Queries) {
    Write-Section "Retrieve : $q"
    Invoke-Ontology --data $DataDir retrieve --top-k $TopK --depth $Depth $q

    Write-Section "Ask ($Provider) : $q"
    Invoke-Ontology --data $DataDir ask --top-k $TopK --depth $Depth @ProviderArgs $q
}

# -- Snapshot + export -------------------------------------------------------

Write-Section "Snapshot"
Invoke-Ontology --data $DataDir snapshot

if ($ExportPath) {
    $exportDir = Split-Path -Parent $ExportPath
    if ($exportDir -and -not (Test-Path $exportDir)) {
        New-Item -ItemType Directory -Path $exportDir | Out-Null
    }
    Write-Section "Export -> $ExportPath"
    Invoke-Ontology --data $DataDir export $ExportPath
}

Write-Host ""
Write-Host "Analyse terminee." -ForegroundColor Green
