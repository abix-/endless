<#
.SYNOPSIS
    Sanitizes text by replacing real values with fake values.
    Reads from stdin, outputs sanitized text.
#>

$secretsPath = "C:\code\secrets.json"
$config = Get-Content $secretsPath -Raw | ConvertFrom-Json

# Read all input
$text = $input | Out-String

# Replace real values with fake values (sorted by length, longest first)
$mappings = $config.mappings.PSObject.Properties | Sort-Object { $_.Name.Length } -Descending

foreach ($m in $mappings) {
    $text = $text -replace [regex]::Escape($m.Name), $m.Value
}

# Output sanitized text
$text
