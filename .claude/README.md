# Claude Code Secret Sanitization System

## The Problem

When you use Claude Code, everything Claude sees gets sent to Anthropic's servers - file contents, command outputs, etc. If your code contains secrets (server names, IPs, API keys), Anthropic sees them too.

## The Solution

Replace real secrets with fake values before Claude sees anything, then restore real values when Claude exits.

```
Your files:     server: real.domain.local
                        |
                        v  (sanitize)
Claude sees:    server: example.test
                        |
                        v  (unsanitize)
Your files:     server: real.domain.local
```

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│                     START CLAUDE CODE                        │
│                            |                                 │
│                            v                                 │
│               SessionStart Hook runs                         │
│                     Sanitize.ps1                             │
│              (real values -> fake values)                    │
└─────────────────────────────────────────────────────────────┘
                            |
                            v
┌─────────────────────────────────────────────────────────────┐
│                     CLAUDE WORKING                           │
│                                                              │
│   Read file  --> Claude sees fake values                     │
│   Grep       --> Claude sees fake values                     │
│   Edit       --> Works with fake values                      │
│                                                              │
│   Bash command:                                              │
│     1. PreToolUse hook unsanitizes files                     │
│     2. Command runs (with real values so code works)         │
│     3. Output piped through SanitizeOutput.ps1               │
│     4. Claude sees sanitized output                          │
│     5. PostToolUse hook re-sanitizes files                   │
│                                                              │
└─────────────────────────────────────────────────────────────┘
                            |
                            v
┌─────────────────────────────────────────────────────────────┐
│                      EXIT CLAUDE CODE                        │
│                            |                                 │
│                            v                                 │
│                   Stop Hook runs                             │
│                    Unsanitize.ps1                            │
│              (fake values -> real values)                    │
│                            |                                 │
│                            v                                 │
│                Code ready to run normally                    │
└─────────────────────────────────────────────────────────────┘
```

## What's Protected

| Vector | Protected? | How |
|--------|-----------|-----|
| Read files | YES | Files sanitized at session start |
| Grep files | YES | Files sanitized at session start |
| Edit files | YES | Works with fake values |
| Bash output | YES | Output piped through sanitizer |
| Git history | PARTIAL | Output sanitized, but old commits could have secrets |

## Limitations

1. **Git history** - If you committed real secrets before, they exist in history
2. **Performance** - Every bash command has overhead (unsanitize -> run -> sanitize)
3. **Binary files** - Only text files in configured extensions are sanitized

---

## Setup Instructions

### Step 1: Create secrets.json

Create `C:\code\secrets.json` (OUTSIDE your project, so it's not committed):

```json
{
  "mappings": {
    "actual-secret-value": "fake-replacement-value",
    "my-server.internal.corp": "server.example.test",
    "192.168.1.50": "10.0.0.99",
    "C:\\real\\path\\to\\secrets": "C:\\fake\\path"
  },
  "fileExtensions": [
    ".yaml", ".yml", ".json", ".ps1", ".config", ".txt", ".md",
    ".js", ".ts", ".py", ".xml", ".ini", ".env", ".sh", ".bat",
    ".cmd", ".cs", ".java", ".go", ".rb", ".php", ".html", ".css", ".sql"
  ],
  "excludePaths": [
    ".git",
    "node_modules",
    ".claude"
  ]
}
```

### Step 2: Create .claude folder structure

```
YourProject\
└── .claude\
    ├── settings.json
    ├── Sanitize.ps1
    ├── Unsanitize.ps1
    ├── SanitizeOutput.ps1
    └── hooks\
        └── RunWrapper.ps1
```

### Step 3: Create settings.json

Create `.claude\settings.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "powershell.exe -ExecutionPolicy Bypass -NoProfile -File C:/code/YourProject/.claude/Sanitize.ps1"
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "powershell.exe -ExecutionPolicy Bypass -NoProfile -File C:/code/YourProject/.claude/Unsanitize.ps1"
      }]
    }],
    "PreToolUse": [{
      "matcher": "Bash",
      "hooks": [{
        "type": "command",
        "command": "powershell.exe -ExecutionPolicy Bypass -NoProfile -File C:/code/YourProject/.claude/hooks/RunWrapper.ps1"
      }]
    }],
    "PostToolUse": [{
      "matcher": "Bash",
      "hooks": [{
        "type": "command",
        "command": "powershell.exe -ExecutionPolicy Bypass -NoProfile -File C:/code/YourProject/.claude/hooks/RunWrapper.ps1"
      }]
    }]
  }
}
```

**IMPORTANT:** Replace `C:/code/YourProject` with your actual project path!

### Step 4: Create Sanitize.ps1

Create `.claude\Sanitize.ps1`:

```powershell
<#
.SYNOPSIS
    Sanitizes project files by replacing real values with fake values.
#>

param(
    [string]$ProjectPath = "C:\code\YourProject",
    [string]$SecretsPath = "C:\code\secrets.json",
    [switch]$Quiet
)

if (-not (Test-Path $SecretsPath)) {
    Write-Error "Secrets file not found: $SecretsPath"
    exit 1
}

$config = Get-Content $SecretsPath -Raw | ConvertFrom-Json
$mappings = $config.mappings
$extensions = $config.fileExtensions
$excludePaths = $config.excludePaths

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

$files = Get-ChildItem -Path $ProjectPath -Recurse -File | Where-Object {
    $file = $_
    $ext = $file.Extension.ToLower()
    if ($extensions -notcontains $ext) { return $false }
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

        foreach ($r in $replacements) {
            $content = $content -replace [regex]::Escape($r.Real), $r.Fake
        }

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
```

### Step 5: Create Unsanitize.ps1

Create `.claude\Unsanitize.ps1`:

```powershell
<#
.SYNOPSIS
    Restores project files by replacing fake values with real values.
#>

param(
    [string]$ProjectPath = "C:\code\YourProject",
    [string]$SecretsPath = "C:\code\secrets.json",
    [switch]$Quiet
)

if (-not (Test-Path $SecretsPath)) {
    Write-Error "Secrets file not found: $SecretsPath"
    exit 1
}

$config = Get-Content $SecretsPath -Raw | ConvertFrom-Json
$mappings = $config.mappings
$extensions = $config.fileExtensions
$excludePaths = $config.excludePaths

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

$files = Get-ChildItem -Path $ProjectPath -Recurse -File | Where-Object {
    $file = $_
    $ext = $file.Extension.ToLower()
    if ($extensions -notcontains $ext) { return $false }
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

        foreach ($r in $replacements) {
            $content = $content -replace [regex]::Escape($r.Fake), $r.Real
        }

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
```

### Step 6: Create SanitizeOutput.ps1

Create `.claude\SanitizeOutput.ps1`:

```powershell
<#
.SYNOPSIS
    Sanitizes text by replacing real values with fake values.
    Reads from stdin, outputs sanitized text.
#>

$secretsPath = "C:\code\secrets.json"
$config = Get-Content $secretsPath -Raw | ConvertFrom-Json

$text = $input | Out-String

$mappings = $config.mappings.PSObject.Properties | Sort-Object { $_.Name.Length } -Descending

foreach ($m in $mappings) {
    $text = $text -replace [regex]::Escape($m.Name), $m.Value
}

$text
```

### Step 7: Create hooks\RunWrapper.ps1

Create `.claude\hooks\RunWrapper.ps1`:

```powershell
<#
.SYNOPSIS
    Hook that wraps Bash execution with unsanitize/sanitize and output sanitization.
#>

param()

$logFile = "C:\code\YourProject\.claude\hooks\debug.log"

$hookData = $input | Out-String | ConvertFrom-Json -ErrorAction SilentlyContinue

$hookEvent = $hookData.hook_event_name
$toolInput = $hookData.tool_input

Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Event: $hookEvent"
Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Tool: $($hookData.tool_name)"

$sanitizeScript = "C:\code\YourProject\.claude\Sanitize.ps1"
$unsanitizeScript = "C:\code\YourProject\.claude\Unsanitize.ps1"
$sanitizeOutputScript = "C:\code\YourProject\.claude\SanitizeOutput.ps1"

$command = $toolInput.command
Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Command: $command"

if (-not $command) {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - No command, exiting"
    exit 0
}

$isDirectSanitizeCall = $command -match '^\s*powershell.*Sanitize\.ps1' -or
                        $command -match '^\s*powershell.*Unsanitize\.ps1'

if ($isDirectSanitizeCall) {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Skipping direct sanitize call"
    exit 0
}

Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Processing $hookEvent for: $command"

if ($hookEvent -eq "PreToolUse") {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Running Unsanitize..."
    & powershell.exe -ExecutionPolicy Bypass -NoProfile -File $unsanitizeScript -Quiet 2>$null
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Unsanitize done"

    $wrappedCommand = "($command) 2>&1 | powershell.exe -ExecutionPolicy Bypass -NoProfile -File '$sanitizeOutputScript'"

    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Wrapped: $wrappedCommand"

    $output = @{
        hookSpecificOutput = @{
            hookEventName = "PreToolUse"
            permissionDecision = "allow"
            updatedInput = @{
                command = $wrappedCommand
            }
        }
    }
    $output | ConvertTo-Json -Depth 5 -Compress
}
elseif ($hookEvent -eq "PostToolUse") {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Running Sanitize..."
    & powershell.exe -ExecutionPolicy Bypass -NoProfile -File $sanitizeScript -Quiet 2>$null
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Sanitize done"
}
```

### Step 8: Update all paths

Replace `C:\code\YourProject` with your actual project path in:
- settings.json
- Sanitize.ps1 (the $ProjectPath default)
- Unsanitize.ps1 (the $ProjectPath default)
- RunWrapper.ps1 (the $logFile and script paths)

### Step 9: Restart Claude Code

The hooks only take effect after restarting Claude Code.

---

## Testing

After setup, test with:

1. Start Claude Code in your project
2. Ask Claude to read a file that contains secrets - should see fake values
3. Ask Claude to run `cat yourfile.yaml` - should see fake values in output
4. Exit Claude Code
5. Check your files - should have real values restored
