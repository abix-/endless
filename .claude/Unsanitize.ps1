<#
.SYNOPSIS
    Restores project files by replacing fake values with real values.
.DESCRIPTION
    Reads mappings from secrets.json and replaces all fake values with real values
    across project files. Run this AFTER finishing a Claude Code session.
.NOTES
    Run with: powershell -ExecutionPolicy Bypass -File Unsanitize.ps1
#>

param(
    [string]$ProjectPath = "C:\code\endless",
    [string]$SecretsPath = "C:\code\secrets.json",
    [switch]$Quiet
)

# Load secrets configuration
if (-not (Test-Path $SecretsPath)) {
    Write-Error "Secrets file not found: $SecretsPath"
    exit 1
}

$config = Get-Content $SecretsPath -Raw | ConvertFrom-Json
$mappings = $config.mappings
$extensions = $config.fileExtensions
$excludePaths = $config.excludePaths

# Build list of fake->real replacements, sorted by length (longest first to avoid partial matches)
$replacements = @()
foreach ($prop in $mappings.PSObject.Properties) {
    $replacements += @{
        Fake = $prop.Value
        Real = $prop.Name
    }
}
$replacements = $replacements | Sort-Object { $_.Fake.Length } -Descending

if (-not $Quiet) {
    Write-Host "Unsanitizing project: $ProjectPath" -ForegroundColor Cyan
    Write-Host "Loaded $($replacements.Count) mappings" -ForegroundColor Cyan
}

# Get all files matching extensions, excluding specified paths
$files = Get-ChildItem -Path $ProjectPath -Recurse -File | Where-Object {
    $file = $_
    $ext = $file.Extension.ToLower()

    # Check extension
    if ($extensions -notcontains $ext) { return $false }

    # Check excluded paths
    foreach ($exclude in $excludePaths) {
        if ($file.FullName -like "*\$exclude\*") { return $false }
    }

    return $true
}

$modifiedCount = 0

foreach ($file in $files) {
    try {
        $content = Get-Content $file.FullName -Raw -ErrorAction Stop
        if ([string]::IsNullOrEmpty($content)) { continue }

        $originalContent = $content

        # Apply all replacements (fake -> real)
        foreach ($r in $replacements) {
            $content = $content -replace [regex]::Escape($r.Fake), $r.Real
        }

        # Only write if changed
        if ($content -ne $originalContent) {
            Set-Content -Path $file.FullName -Value $content -NoNewline -Encoding UTF8
            if (-not $Quiet) { Write-Host "  Restored: $($file.FullName)" -ForegroundColor Green }
            $modifiedCount++
        }
    }
    catch {
        Write-Warning "Failed to process $($file.FullName): $_"
    }
}

if (-not $Quiet) {
    Write-Host ""
    Write-Host "Unsanitization complete. Restored $modifiedCount files." -ForegroundColor Cyan
    Write-Host "Your code is ready to run with real values." -ForegroundColor Yellow
}
