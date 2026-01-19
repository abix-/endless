<#
.SYNOPSIS
    Hook that wraps Bash execution with unsanitize/sanitize and output sanitization.
.DESCRIPTION
    PreToolUse: Unsanitizes files, wraps command to sanitize output
    PostToolUse: Re-sanitizes files
#>

param()

$logFile = "C:\code\endless\.claude\hooks\debug.log"

# Read hook input from stdin
$hookData = $input | Out-String | ConvertFrom-Json -ErrorAction SilentlyContinue

$hookEvent = $hookData.hook_event_name
$toolInput = $hookData.tool_input

Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Event: $hookEvent"
Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Tool: $($hookData.tool_name)"

$sanitizeScript = "C:\code\endless\.claude\Sanitize.ps1"
$unsanitizeScript = "C:\code\endless\.claude\Unsanitize.ps1"
$sanitizeOutputScript = "C:\code\endless\.claude\SanitizeOutput.ps1"

# Get the command from tool_input
$command = $toolInput.command
Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Command: $command"

if (-not $command) {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - No command, exiting"
    exit 0
}

# Skip only direct calls to our sanitize scripts (not wrapped commands)
# Check if command directly runs Sanitize.ps1 or Unsanitize.ps1
$isDirectSanitizeCall = $command -match '^\s*powershell.*Sanitize\.ps1' -or
                        $command -match '^\s*powershell.*Unsanitize\.ps1'

if ($isDirectSanitizeCall) {
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Skipping direct sanitize call"
    exit 0
}

Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Processing $hookEvent for: $command"

if ($hookEvent -eq "PreToolUse") {
    # 1. Unsanitize files so scripts can run with real values
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Running Unsanitize..."
    & powershell.exe -ExecutionPolicy Bypass -NoProfile -File $unsanitizeScript -Quiet 2>$null
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Unsanitize done"

    # 2. Wrap command to pipe output through sanitizer
    # Use bash syntax: (command) 2>&1 | powershell.exe -File SanitizeOutput.ps1
    $wrappedCommand = "($command) 2>&1 | powershell.exe -ExecutionPolicy Bypass -NoProfile -File '$sanitizeOutputScript'"

    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Wrapped: $wrappedCommand"

    # Return JSON to update the command
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
    # Re-sanitize files
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Running Sanitize..."
    & powershell.exe -ExecutionPolicy Bypass -NoProfile -File $sanitizeScript -Quiet 2>$null
    Add-Content -Path $logFile -Value "$(Get-Date -Format 'HH:mm:ss') - Sanitize done"
}
