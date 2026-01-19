<#
.SYNOPSIS
    Sanitizes project files by replacing real values with fake values.
.DESCRIPTION
    Reads mappings from secrets.json and replaces all real values with fake values
    across project files. Run this BEFORE starting a Claude Code session.
.NOTES
    Run with: powershell -ExecutionPolicy Bypass -File Sanitize.ps1
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

# Build list of real->fake replacements, sorted by length (longest first to avoid partial matches)
$replacements = @()
foreach ($prop in $mappings.PSObject.Properties) {
    $replacements += @{
        Real = $prop.Name
        Fake = $prop.Value
    }
}
$replacements = $replacements | Sort-Object { $_.Real.Length } -Descending

if (-not $Quiet) {
    Write-Host "Sanitizing project: $ProjectPath" -ForegroundColor Cyan
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

        # Apply all replacements
        foreach ($r in $replacements) {
            $content = $content -replace [regex]::Escape($r.Real), $r.Fake
        }

        # Only write if changed
        if ($content -ne $originalContent) {
            Set-Content -Path $file.FullName -Value $content -NoNewline -Encoding UTF8
            if (-not $Quiet) { Write-Host "  Sanitized: $($file.FullName)" -ForegroundColor Green }
            $modifiedCount++
        }
    }
    catch {
        Write-Warning "Failed to process $($file.FullName): $_"
    }
}

if (-not $Quiet) {
    Write-Host ""
    Write-Host "Sanitization complete. Modified $modifiedCount files." -ForegroundColor Cyan
    Write-Host "Run Unsanitize.ps1 when done to restore real values." -ForegroundColor Yellow
}
