function Get-SecretMap {
    <#
    .SYNOPSIS
        Loads secret mappings from user config.
    .EXAMPLE
        $map = Get-SecretMap
    #>
    [CmdletBinding()]
    param()

    $path = 'C:\code\secrets.json'
    if (-not (Test-Path $path)) {
        return [PSCustomObject]@{ secrets = [PSCustomObject]@{} }
    }
    Get-Content $path -Raw | ConvertFrom-Json
}

function Convert-SecretContent {
    <#
    .SYNOPSIS
        Replaces secrets with placeholders or vice versa.
    .EXAMPLE
        Convert-SecretContent -Content $text -Mode Sanitize
    .EXAMPLE
        Convert-SecretContent -Content $text -Mode Restore
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Content,
        [Parameter(Mandatory)][ValidateSet('Sanitize','Restore')][string]$Mode
    )

    $map = Get-SecretMap
    $result = $Content

    foreach ($s in $map.secrets.PSObject.Properties) {
        $placeholder = "{{$($s.Name)}}"
        $result = switch ($Mode) {
            'Sanitize' { $result.Replace($s.Value, $placeholder) }
            'Restore'  { $result.Replace($placeholder, $s.Value) }
        }
    }

    [PSCustomObject]@{ Content = $result; Changed = $result -ne $Content }
}

# Main - process hook input
$hook = $input | Out-String | ConvertFrom-Json

# Debug logging
$logFile = Join-Path $env:TEMP 'claude-sanitizer.log'
"$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') - Tool: $($hook.tool_name)" | Add-Content $logFile
$tool = $hook.tool_name
$event = $hook.hook_event_name

$output = switch ($event) {
    'PreToolUse' {
        switch ($tool) {
            'Read' {
                $file = $hook.tool_input.file_path
                if (-not $file -or -not (Test-Path $file -PathType Leaf)) {
                    [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow' } }
                    break
                }

                $converted = Convert-SecretContent -Content (Get-Content $file -Raw) -Mode Sanitize
                if (-not $converted.Changed) {
                    [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow' } }
                    break
                }

                $tempDir = Join-Path $env:TEMP 'claude-sanitized'
                New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
                $tempFile = Join-Path $tempDir (Split-Path $file -Leaf)
                Set-Content $tempFile $converted.Content -NoNewline

                [PSCustomObject]@{
                    hookSpecificOutput = [PSCustomObject]@{
                        hookEventName = 'PreToolUse'
                        permissionDecision = 'allow'
                        updatedInput = [PSCustomObject]@{ file_path = $tempFile }
                    }
                }
            }
            'Grep' {
                $searchPath = $hook.tool_input.path
                if (-not $searchPath) { $searchPath = Get-Location }

                $tempBase = Join-Path $env:TEMP 'claude-sanitized-grep'
                if (Test-Path $tempBase) { Remove-Item $tempBase -Recurse -Force }
                New-Item -ItemType Directory -Path $tempBase -Force | Out-Null

                # Get files to process
                $files = if (Test-Path $searchPath -PathType Leaf) {
                    @(Get-Item $searchPath)
                } else {
                    Get-ChildItem $searchPath -File -Recurse -ErrorAction SilentlyContinue
                }

                foreach ($file in $files) {
                    try {
                        $content = Get-Content $file.FullName -Raw -ErrorAction Stop
                        $converted = Convert-SecretContent -Content $content -Mode Sanitize

                        # Preserve directory structure
                        $relativePath = $file.FullName.Replace($searchPath, '').TrimStart('\', '/')
                        $tempFile = Join-Path $tempBase $relativePath
                        $tempFileDir = Split-Path $tempFile -Parent
                        if ($tempFileDir -and -not (Test-Path $tempFileDir)) {
                            New-Item -ItemType Directory -Path $tempFileDir -Force | Out-Null
                        }

                        Set-Content $tempFile $converted.Content -NoNewline -ErrorAction Stop
                    } catch {
                        # Skip binary/unreadable files
                    }
                }

                $updatedInput = @{}
                $hook.tool_input.PSObject.Properties | ForEach-Object { $updatedInput[$_.Name] = $_.Value }
                $updatedInput['path'] = $tempBase

                [PSCustomObject]@{
                    hookSpecificOutput = [PSCustomObject]@{
                        hookEventName = 'PreToolUse'
                        permissionDecision = 'allow'
                        updatedInput = [PSCustomObject]$updatedInput
                    }
                }
            }
            'Edit' {
                $filePath = $hook.tool_input.file_path
                $oldString = $hook.tool_input.old_string
                $newString = $hook.tool_input.new_string

                # Check if old_string contains placeholders
                $oldConverted = Convert-SecretContent -Content $oldString -Mode Restore
                $newConverted = Convert-SecretContent -Content $newString -Mode Restore

                if ($oldConverted.Changed) {
                    # Placeholder in old_string - do edit ourselves, then make Edit a no-op
                    $fileContent = Get-Content $filePath -Raw
                    $updatedContent = $fileContent.Replace($oldConverted.Content, $newConverted.Content)

                    if ($updatedContent -eq $fileContent) {
                        # old_string not found even after conversion
                        [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'deny'; permissionDecisionReason = 'String to replace not found in file.' } }
                    } else {
                        # Do the edit ourselves
                        Set-Content $filePath $updatedContent -NoNewline
                        # Return allow with old_string=new_string so Edit becomes a no-op
                        $updated = @{}
                        $hook.tool_input.PSObject.Properties | ForEach-Object { $updated[$_.Name] = $_.Value }
                        $updated['old_string'] = $newConverted.Content
                        $updated['new_string'] = $newConverted.Content
                        [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow'; updatedInput = [PSCustomObject]$updated } }
                    }
                } else {
                    # No placeholder - let Edit run normally, but restore new_string if needed
                    if ($newConverted.Changed) {
                        $updated = @{}
                        $hook.tool_input.PSObject.Properties | ForEach-Object { $updated[$_.Name] = $_.Value }
                        $updated['new_string'] = $newConverted.Content
                        [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow'; updatedInput = [PSCustomObject]$updated } }
                    } else {
                        [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow' } }
                    }
                }
            }
            'Write' {
                $updated = @{}
                $hook.tool_input.PSObject.Properties | ForEach-Object { $updated[$_.Name] = $_.Value }
                $changed = $false

                if ($hook.tool_input.content) {
                    $converted = Convert-SecretContent -Content $hook.tool_input.content -Mode Restore
                    if ($converted.Changed) {
                        $updated['content'] = $converted.Content
                        $changed = $true
                    }
                }

                if ($changed) {
                    [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow'; updatedInput = [PSCustomObject]$updated } }
                } else {
                    [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow' } }
                }
            }
            default { [PSCustomObject]@{ hookSpecificOutput = [PSCustomObject]@{ hookEventName = 'PreToolUse'; permissionDecision = 'allow' } } }
        }
    }
    'PostToolUse' {
        if ($tool -eq 'Bash' -and $hook.tool_response) {
            $converted = Convert-SecretContent -Content $hook.tool_response -Mode Sanitize
            if ($converted.Changed) {
                [PSCustomObject]@{ additionalContext = '[Secrets sanitized]' }
            }
        }
    }
}

if ($output) { $output | ConvertTo-Json -Compress -Depth 10 }
