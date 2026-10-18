<#
  Steps of the Windows updater verification (.github/workflows/updater-windows.yml).
  Runs on a GitHub-hosted windows runner under PowerShell 7. Never publishes anything.

  Usage:  pwsh scripts/verify-updater-windows.ps1 -Step <step> [-OldTag v0.3.1] [-NewTag v0.3.2]

  Steps, in the order the workflow runs them:
    resolve       decide which two published releases are compared; write state
    download-old download the OLD NSIS setup + its .sha256 and verify the hash
    install-old   install it silently (/S, default per-user location), record the installed version
    markers       create user-data files under %APPDATA%\dev.genius.studio
    verify-new    after the Rust test ran the NEW installer silently: version, data, location
    launch        start the installed app, wait, check the process and the window title, stop it
    collect       gather logs for the artifact upload (never fails the job)

  Every check writes a ::notice:: (passed) or ::error:: (failed, exit 1) annotation. Facts are
  reported as found: nothing here is assumed to have worked.
#>
param(
  [Parameter(Mandatory = $true)]
  [ValidateSet('resolve', 'download-old', 'install-old', 'markers', 'verify-new', 'launch', 'collect')]
  [string]$Step,
  [string]$OldTag = '',
  [string]$NewTag = '',
  [string]$Repo = $env:GITHUB_REPOSITORY,
  [string]$ResultFile = '',
  [string]$ExpectedTitle = 'Genius',
  [string]$Identifier = 'dev.genius.studio'
)

$ErrorActionPreference = 'Stop'
$Work = Join-Path $env:RUNNER_TEMP 'updater-verify'
$StateFile = Join-Path $Work 'state.json'
New-Item -ItemType Directory -Force -Path $Work | Out-Null

# ------------------------------------------------------------------ helpers

function Escape-Annotation([string]$text) {
  return ($text -replace '%', '%25' -replace "`r", '' -replace "`n", '%0A')
}
function Notice([string]$title, [string]$msg) {
  Write-Host "::notice title=$(Escape-Annotation $title)::$(Escape-Annotation $msg)"
}
function Fail([string]$title, [string]$msg) {
  Write-Host "::error title=$(Escape-Annotation $title)::$(Escape-Annotation $msg)"
  Write-Host "FAILED: $title - $msg"
  exit 1
}
function Get-State {
  if (Test-Path $StateFile) { return Get-Content $StateFile -Raw | ConvertFrom-Json -AsHashtable }
  return @{}
}
function Set-State([string]$key, $value) {
  $s = Get-State
  $s[$key] = $value
  $s | ConvertTo-Json -Depth 6 | Set-Content -Path $StateFile -Encoding utf8
}
function Add-Output([string]$name, [string]$value) {
  if ($env:GITHUB_OUTPUT) { Add-Content -Path $env:GITHUB_OUTPUT -Value "$name=$value" -Encoding utf8 }
}
function Add-Summary([string]$markdown) {
  if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $markdown -Encoding utf8 }
}
function Versionize([string]$tag) {
  # "v0.3.2" -> "0.3.2"
  return $tag.Trim().TrimStart('v', 'V')
}
function Core-Version([string]$v) {
  # First three numeric components of a file/product version ("0.3.2.0" -> "0.3.2").
  $m = [regex]::Match([string]$v, '^\d+\.\d+\.\d+')
  if ($m.Success) { return $m.Value }
  return [string]$v
}
function Sha256-Of([string]$path) {
  return (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant()
}
function Read-Published-Hash([string]$shaFile) {
  $token = ((Get-Content $shaFile -Raw) -split '\s+' | Where-Object { $_ })[0]
  if ($token -notmatch '^[0-9a-fA-F]{64}$') { Fail 'Checksum file' "$shaFile is not a SHA-256 file: '$token'" }
  return $token.ToLowerInvariant()
}

# Locate the installed app through the uninstall registry entries NSIS writes.
function Find-Install {
  $roots = @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
  )
  foreach ($root in $roots) {
    if (-not (Test-Path $root)) { continue }
    foreach ($key in Get-ChildItem $root -ErrorAction SilentlyContinue) {
      $p = Get-ItemProperty -Path $key.PSPath -ErrorAction SilentlyContinue
      if ($null -eq $p -or -not $p.DisplayName -or $p.DisplayName -notlike 'Genius*') { continue }
      $dir = ''
      if ($p.InstallLocation) { $dir = ([string]$p.InstallLocation).Trim('"').TrimEnd('\') }
      if (-not $dir -and $p.UninstallString) {
        $u = ([string]$p.UninstallString).Trim().Trim('"')
        $dir = Split-Path -Parent $u
      }
      if (-not $dir -or -not (Test-Path $dir)) { continue }
      $exes = @(Get-ChildItem -Path $dir -Filter '*.exe' -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -notlike 'uninst*' -and $_.Name -notlike 'unins*' })
      $app = $exes | Where-Object { $_.Name -like 'genius*' } | Select-Object -First 1
      if (-not $app) { $app = $exes | Sort-Object Length -Descending | Select-Object -First 1 }
      if (-not $app) { continue }
      return @{
        Dir            = $dir
        Exe            = $app.FullName
        RegistryKey    = $key.PSPath
        DisplayName    = [string]$p.DisplayName
        DisplayVersion = [string]$p.DisplayVersion
        Scope          = $(if ($root -like 'HKCU:*') { 'per-user (HKCU)' } else { 'per-machine (HKLM)' })
      }
    }
  }
  return $null
}
function Dump-Install-Diagnostics {
  Write-Host '--- diagnostics: uninstall entries mentioning Genius'
  foreach ($root in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall') {
    if (Test-Path $root) {
      Get-ChildItem $root | ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DisplayName -like '*enius*' } | Format-List DisplayName, DisplayVersion, InstallLocation, UninstallString | Out-String | Write-Host
    }
  }
  Write-Host "--- $env:LOCALAPPDATA"
  Get-ChildItem $env:LOCALAPPDATA -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name | Write-Host
  Write-Host "--- $env:LOCALAPPDATA\Programs"
  Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Programs') -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name | Write-Host
}
function Exe-Versions([string]$exe) {
  $vi = (Get-Item $exe).VersionInfo
  return @{ FileVersion = [string]$vi.FileVersion; ProductVersion = [string]$vi.ProductVersion; ProductName = [string]$vi.ProductName }
}
function Stop-InstalledApp([string]$dir) {
  $running = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$dir*" })
  foreach ($p in $running) {
    Notice 'App was running' "Stopping $($p.ProcessName) (pid $($p.Id)) that the installer left running"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  }
  if ($running.Count -gt 0) { Start-Sleep -Seconds 2 }
}
function Data-Dir { return (Join-Path $env:APPDATA $Identifier) }

# ------------------------------------------------------------------ steps

switch ($Step) {

  'resolve' {
    if (-not $Repo) { Fail 'Resolve releases' 'GITHUB_REPOSITORY is not set' }
    $listJson = gh api "repos/$Repo/releases?per_page=50" --jq '[.[]|select(.draft==false and .prerelease==false)|.tag_name]'
    if ($LASTEXITCODE -ne 0) { Fail 'Resolve releases' 'gh api could not list the releases' }
    $tags = @($listJson | ConvertFrom-Json)
    if ($tags.Count -lt 1) { Fail 'Resolve releases' 'the repository has no published releases' }
    $new = $NewTag.Trim()
    if (-not $new) { $new = (gh api "repos/$Repo/releases/latest" --jq '.tag_name').Trim() }
    if ($tags -notcontains $new) { Fail 'Resolve releases' "new_tag '$new' is not a published (non-draft) release. Published: $($tags -join ', ')" }
    $old = $OldTag.Trim()
    if (-not $old) {
      $i = [array]::IndexOf($tags, $new)
      if ($i + 1 -ge $tags.Count) { Fail 'Resolve releases' "there is no release older than $new to update from. Pass old_tag explicitly." }
      $old = $tags[$i + 1]
    }
    if ($tags -notcontains $old) { Fail 'Resolve releases' "old_tag '$old' is not a published (non-draft) release. Published: $($tags -join ', ')" }
    if ($old -eq $new) { Fail 'Resolve releases' 'old_tag and new_tag are the same release' }
    $ov = Versionize $old
    $nv = Versionize $new
    try { $cmp = ([version]$ov).CompareTo([version]$nv) } catch { Fail 'Resolve releases' "tags are not plain versions: $old / $new" }
    if ($cmp -ge 0) { Fail 'Resolve releases' "old_tag $old is not older than new_tag $new" }
    Set-State 'oldTag' $old; Set-State 'newTag' $new; Set-State 'oldVersion' $ov; Set-State 'newVersion' $nv; Set-State 'repo' $Repo
    Add-Output 'old_tag' $old; Add-Output 'new_tag' $new; Add-Output 'old_version' $ov; Add-Output 'new_version' $nv
    Notice 'Releases under test' "update from $old to $new in $Repo"
  }

  'download-old' {
    $s = Get-State
    $dir = Join-Path $Work 'old'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    gh release download $s.oldTag -R $s.repo -p '*-setup.exe' -p '*-setup.exe.sha256' -D $dir --clobber
    if ($LASTEXITCODE -ne 0) { Fail 'Download old installer' "gh release download failed for $($s.oldTag)" }
    $exes = @(Get-ChildItem $dir -Filter '*-setup.exe' -File)
    if ($exes.Count -ne 1) { Fail 'Download old installer' "expected exactly one *-setup.exe in $($s.oldTag), found $($exes.Count)" }
    $exe = $exes[0]
    $shaFile = "$($exe.FullName).sha256"
    if (-not (Test-Path $shaFile)) { Fail 'Download old installer' "the release $($s.oldTag) publishes no $($exe.Name).sha256" }
    $published = Read-Published-Hash $shaFile
    $actual = Sha256-Of $exe.FullName
    if ($actual -ne $published) { Fail 'Old installer checksum' "SHA-256 of $($exe.Name) is $actual but the release publishes $published" }
    $head = [System.IO.File]::ReadAllBytes($exe.FullName)[0..1]
    if ($exe.Length -lt 1MB -or $head[0] -ne 0x4D -or $head[1] -ne 0x5A) { Fail 'Old installer' "$($exe.Name) ($($exe.Length) bytes) is not a Windows executable" }
    $sig = Get-AuthenticodeSignature -FilePath $exe.FullName
    Set-State 'oldInstaller' $exe.FullName; Set-State 'oldInstallerSha256' $actual; Set-State 'oldInstallerSignature' ([string]$sig.Status)
    Notice 'Old installer verified' "$($exe.Name), $($exe.Length) bytes, SHA-256 $actual equals the published checksum. Authenticode status: $($sig.Status) (the installer is expected to be NotSigned)"
  }

  'install-old' {
    $s = Get-State
    $before = Find-Install
    if ($before) { Notice 'Pre-existing install' "Genius is already installed at $($before.Dir); the result below may be affected" }
    # NSIS silent install into the default location. Tauri's default install mode is per-user.
    $p = Start-Process -FilePath $s.oldInstaller -ArgumentList '/S' -PassThru
    $null = $p.Handle
    if (-not $p.WaitForExit(300000)) { Stop-Process -Id $p.Id -Force; Fail 'Install old version' 'the silent installer did not finish within 5 minutes' }
    if ($p.ExitCode -ne 0) { Fail 'Install old version' "the silent installer exited with code $($p.ExitCode)" }
    $deadline = (Get-Date).AddSeconds(60)
    $inst = $null
    while ((Get-Date) -lt $deadline -and -not $inst) { $inst = Find-Install; if (-not $inst) { Start-Sleep -Seconds 2 } }
    if (-not $inst) { Dump-Install-Diagnostics; Fail 'Install old version' 'the installer exited with 0 but no Genius uninstall entry/executable was found' }
    Stop-InstalledApp $inst.Dir
    $v = Exe-Versions $inst.Exe
    Write-Host "Installed: $($inst.Exe)  FileVersion=$($v.FileVersion) ProductVersion=$($v.ProductVersion) ProductName=$($v.ProductName) DisplayVersion=$($inst.DisplayVersion) scope=$($inst.Scope)"
    if ((Core-Version $v.ProductVersion) -ne $s.oldVersion -and (Core-Version $v.FileVersion) -ne $s.oldVersion) {
      Fail 'Old version installed' "the installed executable reports FileVersion '$($v.FileVersion)' / ProductVersion '$($v.ProductVersion)', expected $($s.oldVersion)"
    }
    $s2 = @{ dir = $inst.Dir; exe = $inst.Exe; versions = $v; displayVersion = $inst.DisplayVersion; scope = $inst.Scope; exeSha256 = (Sha256-Of $inst.Exe) }
    Set-State 'oldInstall' $s2
    Notice 'Old version installed' "$($inst.Exe) is version $($v.FileVersion) ($($inst.Scope)), as expected for $($s.oldTag)"
    Add-Output 'install_dir' $inst.Dir
  }

  'markers' {
    $data = Data-Dir
    $sub = Join-Path $data 'update-verify'
    New-Item -ItemType Directory -Force -Path (Join-Path $sub 'projects\demo') | Out-Null
    $token = [guid]::NewGuid().ToString()
    $files = @{
      (Join-Path $sub 'marker.txt')                = "genius-updater-verification $token`n"
      (Join-Path $data 'update-verify-settings.json') = (@{ theme = 'dark'; fakeSetting = $token; note = 'created by the updater verification; not a real Genius setting' } | ConvertTo-Json)
      (Join-Path $sub 'projects\demo\main.tsx')    = "export const demo = '$token';`n"
    }
    $hashes = @{}
    foreach ($path in $files.Keys) {
      Set-Content -Path $path -Value $files[$path] -Encoding utf8 -NoNewline
      $hashes[$path] = Sha256-Of $path
    }
    Set-State 'markers' $hashes
    Notice 'User data markers' "created $($hashes.Count) files under $data"
  }

  'verify-new' {
    $s = Get-State
    $res = $null
    if ($ResultFile -and (Test-Path $ResultFile)) { $res = Get-Content $ResultFile -Raw | ConvertFrom-Json }
    if (-not $res) { Fail 'Updater test result' "the Rust test wrote no result file ($ResultFile)" }
    if ($res.installerExitCode -ne 0) { Fail 'New installer' "silent installer exit code was '$($res.installerExitCode)'" }
    if ($res.newVersion -ne $s.newVersion) { Fail 'New version' "the updater selected $($res.newVersion) but the workflow targets $($s.newVersion)" }

    # Independent check of what the Rust test downloaded and ran.
    if (Test-Path $res.downloadedPath) {
      $dl = Sha256-Of $res.downloadedPath
      if ($dl -ne $res.sha256) { Fail 'Downloaded installer' "file on disk has SHA-256 $dl, the updater verified $($res.sha256)" }
    }
    $published = (gh release download $s.newTag -R $s.repo -p "$($res.asset).sha256" -O - 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -eq 0 -and $published) {
      $pub = ($published -split '\s+')[0].ToLowerInvariant()
      if ($pub -ne $res.sha256) { Fail 'Downloaded installer' "published checksum of $($res.asset) is $pub but the updater verified $($res.sha256)" }
      Notice 'New installer checksum' "$($res.asset) SHA-256 $pub matches the release's published checksum"
    } else {
      Notice 'New installer checksum' 'could not re-read the published checksum independently (the Rust test already compared it)'
    }

    $deadline = (Get-Date).AddSeconds(60)
    $inst = $null
    $ver = ''
    while ((Get-Date) -lt $deadline) {
      $inst = Find-Install
      if ($inst) { $ver = Core-Version (Exe-Versions $inst.Exe).FileVersion; if ($ver -eq $s.newVersion) { break } }
      Start-Sleep -Seconds 2
    }
    if (-not $inst) { Dump-Install-Diagnostics; Fail 'Updated install' 'Genius is no longer installed after running the new installer' }
    Stop-InstalledApp $inst.Dir
    $v = Exe-Versions $inst.Exe
    Write-Host "After update: $($inst.Exe)  FileVersion=$($v.FileVersion) ProductVersion=$($v.ProductVersion) DisplayVersion=$($inst.DisplayVersion)"
    if ((Core-Version $v.FileVersion) -ne $s.newVersion -and (Core-Version $v.ProductVersion) -ne $s.newVersion) {
      Fail 'Updated version' "installed executable still reports FileVersion '$($v.FileVersion)' / ProductVersion '$($v.ProductVersion)', expected $($s.newVersion)"
    }
    if ($inst.DisplayVersion -and (Core-Version $inst.DisplayVersion) -ne $s.newVersion) {
      Fail 'Updated registry entry' "the uninstall entry says DisplayVersion '$($inst.DisplayVersion)', expected $($s.newVersion)"
    }
    if ($inst.Dir -ne $s.oldInstall.dir) { Fail 'Install location' "the update installed to $($inst.Dir) but the old version was in $($s.oldInstall.dir)" }
    if ((Sha256-Of $inst.Exe) -eq $s.oldInstall.exeSha256) { Fail 'Updated executable' 'the executable file is byte-identical to the old version' }
    $uninst = @(Get-ChildItem -Path $inst.Dir -Filter 'uninst*.exe' -File -ErrorAction SilentlyContinue)
    if ($uninst.Count -lt 1) { Fail 'Uninstaller' "no uninstaller found in $($inst.Dir) after the update" }

    $markers = $s.markers
    foreach ($path in $markers.Keys) {
      if (-not (Test-Path $path)) { Fail 'User data' "$path was removed by the update" }
      if ((Sha256-Of $path) -ne $markers[$path]) { Fail 'User data' "$path was modified by the update" }
    }
    Set-State 'newInstall' @{ dir = $inst.Dir; exe = $inst.Exe; versions = $v; displayVersion = $inst.DisplayVersion }
    Notice 'Update installed' "$($s.oldTag) -> $($s.newTag): $($inst.Exe) now reports $($v.FileVersion), same folder, uninstaller present"
    Notice 'User data preserved' "$($markers.Count) marker files in $(Data-Dir) are unchanged"
  }

  'launch' {
    $s = Get-State
    $exe = $s.newInstall.exe
    if (-not $exe -or -not (Test-Path $exe)) { Fail 'Launch' 'no installed executable recorded' }
    $proc = Start-Process -FilePath $exe -PassThru
    Write-Host "Started $exe (pid $($proc.Id)); waiting 20 seconds"
    Start-Sleep -Seconds 20
    $proc.Refresh()
    if ($proc.HasExited) { Fail 'App start' "the app exited within 20 s with code $($proc.ExitCode)" }
    $title = ''
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline) {
      $proc.Refresh()
      if ($proc.HasExited) { Fail 'App start' "the app exited with code $($proc.ExitCode) while waiting for its window" }
      $title = $proc.MainWindowTitle
      if ($title) { break }
      Start-Sleep -Seconds 2
    }
    try {
      Add-Type -AssemblyName System.Windows.Forms, System.Drawing
      $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
      $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
      $bmp.Save((Join-Path $Work 'app-screenshot.png'))
      $g.Dispose(); $bmp.Dispose()
    } catch { Write-Host "screenshot failed: $($_.Exception.Message)" }
    Get-Process | Where-Object { $_.ProcessName -like 'genius*' -or $_.ProcessName -like 'msedgewebview2*' } | Select-Object Id, ProcessName, MainWindowTitle | Format-Table | Out-String | Write-Host
    $alive = -not $proc.HasExited
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Stop-InstalledApp $s.newInstall.dir
    if (-not $alive) { Fail 'App start' 'the app process was not running after the wait' }
    if ($title -ne $ExpectedTitle) { Fail 'App window' "the main window title is '$title', expected '$ExpectedTitle' (see app-screenshot.png in the artifact)" }
    foreach ($path in $s.markers.Keys) {
      if (-not (Test-Path $path)) { Fail 'User data' "$path disappeared after the updated app started" }
    }
    Notice 'Updated app runs' "$exe stayed alive for the whole wait and its main window is titled '$title'"
  }

  'collect' {
    $out = Join-Path $Work 'collected'
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    try {
      $logs = Join-Path (Data-Dir) 'logs'
      if (Test-Path $logs) { Copy-Item -Recurse -Force $logs (Join-Path $out 'app-logs') }
      $dl = Join-Path ([System.IO.Path]::GetTempPath()) 'genius-update'
      if (Test-Path $dl) { Get-ChildItem $dl | Select-Object Name, Length, LastWriteTime | Format-Table | Out-String | Set-Content (Join-Path $out 'genius-update-folder.txt') }
      if (Test-Path $StateFile) { Copy-Item $StateFile (Join-Path $out 'state.json') }
      Dump-Install-Diagnostics *> (Join-Path $out 'install-diagnostics.txt')
      $shot = Join-Path $Work 'app-screenshot.png'
      if (Test-Path $shot) { Copy-Item $shot $out }
    } catch { Write-Host "collect: $($_.Exception.Message)" }
    exit 0
  }
}
