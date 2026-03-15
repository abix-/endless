# === AppX Packages ===
$packages = @(
    'Microsoft.XboxGameCallableUI',
    'Microsoft.OfficePushNotificationUtility',
    'Microsoft.Office.ActionsServer',
    'Microsoft.Windows.PeopleExperienceHost',
    'Microsoft.Windows.CallingShellApp'
)

foreach ($pkg in $packages) {
    Write-Host "Removing $pkg..." -ForegroundColor Yellow
    Get-AppxProvisionedPackage -Online | Where-Object DisplayName -eq $pkg | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue
    Get-AppxPackage -Name $pkg | Remove-AppxPackage -ErrorAction SilentlyContinue
    Get-AppxPackage -AllUsers -Name $pkg | Remove-AppxPackage -AllUsers -ErrorAction SilentlyContinue
    Write-Host "  Done: $pkg"
}

# === Scheduled Tasks ===
$tasks = @(
    @{ Path = '\Microsoft\Windows\Customer Experience Improvement Program\'; Name = 'Consolidator' },
    @{ Path = '\Microsoft\Windows\Customer Experience Improvement Program\'; Name = 'UsbCeip' },
    @{ Path = '\Microsoft\Windows\Feedback\Siuf\'; Name = 'DmClient' },
    @{ Path = '\Microsoft\Windows\Feedback\Siuf\'; Name = 'DmClientOnScenarioDownload' },
    @{ Path = '\Microsoft\Windows\Maps\'; Name = 'MapsToastTask' },
    @{ Path = '\Microsoft\Windows\PushToInstall\'; Name = 'Registration' },
    @{ Path = '\Microsoft\Windows\PushToInstall\'; Name = 'LoginCheck' },
    @{ Path = '\Microsoft\Windows\Windows Error Reporting\'; Name = 'QueueReporting' },
    @{ Path = '\Microsoft\Windows\DUSM\'; Name = 'dusmtask' },
    @{ Path = '\Microsoft\Windows\Application Experience\'; Name = 'Microsoft Compatibility Appraiser' },
    @{ Path = '\Microsoft\Windows\Application Experience\'; Name = 'ProgramDataUpdater' },
    @{ Path = '\Microsoft\Windows\SettingSync\'; Name = 'BackgroundUploadTask' }
)

foreach ($t in $tasks) {
    Write-Host "Disabling $($t.Name)..." -ForegroundColor Yellow
    Disable-ScheduledTask -TaskPath $t.Path -TaskName $t.Name -ErrorAction SilentlyContinue | Out-Null
    Write-Host "  Done: $($t.Name)"
}

Write-Host "`nDebloat complete." -ForegroundColor Green
