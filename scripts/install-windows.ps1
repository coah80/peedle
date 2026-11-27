param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\Peedle"),
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetExe = Join-Path $RepoRoot "target\release\peedle.exe"

if (-not $SkipBuild -or -not (Test-Path $TargetExe)) {
    Push-Location $RepoRoot
    try {
        & cargo build --release --bin peedle
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$DestExe = Join-Path $InstallDir "peedle.exe"
$DestIco = Join-Path $InstallDir "peedle.ico"

if (Test-Path $DestExe) {
    $OldExe = Join-Path $InstallDir "peedle.old.exe"
    if (Test-Path $OldExe) {
        Remove-Item -Force $OldExe -ErrorAction SilentlyContinue
    }
    try {
        Remove-Item -Force $DestExe -ErrorAction Stop
    } catch {
        Move-Item -Force $DestExe $OldExe
    }
}

Copy-Item -Force $TargetExe $DestExe
$IcoProc = Start-Process -FilePath $DestExe -ArgumentList @("--write-ico", $DestIco) -Wait -PassThru -NoNewWindow
if ($IcoProc.ExitCode -ne 0 -or -not (Test-Path $DestIco)) {
    throw "failed to generate peedle.ico (exit $($IcoProc.ExitCode))"
}

$Shell = New-Object -ComObject WScript.Shell
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$Shortcuts = @(
    (Join-Path $StartMenuDir "Peedle.lnk"),
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "Peedle.lnk")
)

foreach ($LnkPath in $Shortcuts) {
    $Sc = $Shell.CreateShortcut($LnkPath)
    $Sc.TargetPath = $DestExe
    $Sc.WorkingDirectory = $InstallDir
    $Sc.IconLocation = "$DestIco,0"
    $Sc.Description = "Peedle Web Browser"
    $Sc.Save()
}

$ProgId = "PeedleHTML"
$RegBase = "HKCU:\Software"

New-Item -Path "$RegBase\Classes\$ProgId\DefaultIcon" -Force | Out-Null
New-Item -Path "$RegBase\Classes\$ProgId\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "$RegBase\Classes\$ProgId" -Name "(default)" -Value "Peedle HTML Document"
Set-ItemProperty -Path "$RegBase\Classes\$ProgId" -Name "URL Protocol" -Value ""
Set-ItemProperty -Path "$RegBase\Classes\$ProgId\DefaultIcon" -Name "(default)" -Value "$DestIco,0"
Set-ItemProperty -Path "$RegBase\Classes\$ProgId\shell\open\command" -Name "(default)" -Value "`"$DestExe`" `"%1`""

$ClientKey = "$RegBase\Clients\StartMenuInternet\Peedle"
New-Item -Path "$ClientKey\DefaultIcon" -Force | Out-Null
New-Item -Path "$ClientKey\shell\open\command" -Force | Out-Null
New-Item -Path "$ClientKey\Capabilities\URLAssociations" -Force | Out-Null
New-Item -Path "$ClientKey\Capabilities\FileAssociations" -Force | Out-Null

Set-ItemProperty -Path $ClientKey -Name "(default)" -Value "Peedle"
Set-ItemProperty -Path "$ClientKey\DefaultIcon" -Name "(default)" -Value "$DestIco,0"
Set-ItemProperty -Path "$ClientKey\shell\open\command" -Name "(default)" -Value "`"$DestExe`""
Set-ItemProperty -Path "$ClientKey\Capabilities" -Name "ApplicationName" -Value "Peedle"
Set-ItemProperty -Path "$ClientKey\Capabilities" -Name "ApplicationDescription" -Value "Peedle Web Browser"
Set-ItemProperty -Path "$ClientKey\Capabilities" -Name "ApplicationIcon" -Value "$DestIco,0"

foreach ($Scheme in @("http", "https", "peedle")) {
    Set-ItemProperty -Path "$ClientKey\Capabilities\URLAssociations" -Name $Scheme -Value $ProgId
}
foreach ($Ext in @(".htm", ".html", ".xhtml", ".svg")) {
    Set-ItemProperty -Path "$ClientKey\Capabilities\FileAssociations" -Name $Ext -Value $ProgId
}

New-Item -Path "$RegBase\RegisteredApplications" -Force | Out-Null
Set-ItemProperty -Path "$RegBase\RegisteredApplications" -Name "Peedle" -Value "Software\Clients\StartMenuInternet\Peedle\Capabilities"

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PeedleShellNotify {
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(int wEventId, uint uFlags, IntPtr dwItem1, IntPtr dwItem2);
}
"@
[PeedleShellNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

Write-Output "Installed Peedle to $DestExe"
