param(
  [switch]$Force,
  [switch]$NoModifyPath,
  [string]$Version = $env:CHAT2DB_CLI_VERSION,
  [string]$BinDir = $env:CHAT2DB_CLI_BIN_DIR,
  [string]$ManifestBase = $env:CHAT2DB_CLI_MANIFEST_BASE,
  [string]$CdnBase = $env:CHAT2DB_CLI_CDN_BASE
)

$ErrorActionPreference = "Stop"

function Get-ValidatedUrlBase {
  param(
    [Parameter(Mandatory = $true)][string]$Value,
    [Parameter(Mandatory = $true)][string]$Name,
    [switch]$RequireOrigin
  )

  if ($Value -cne $Value.Trim()) {
    throw "$Name is not a canonical URL base: $Value"
  }
  $Value = $Value.TrimEnd("/")
  if ($Value.Contains("\") -or $Value.Contains("%")) {
    throw "$Name is not a canonical URL base: $Value"
  }

  $uri = $null
  if (-not [System.Uri]::TryCreate($Value, [System.UriKind]::Absolute, [ref]$uri)) {
    throw "$Name is not an absolute URL: $Value"
  }
  if ($uri.Scheme -cne "https" -and ($uri.Scheme -cne "http" -or -not $uri.IsLoopback)) {
    throw "$Name must use https; http is permitted only for loopback tests"
  }
  if (-not [string]::IsNullOrEmpty($uri.UserInfo) -or
      -not [string]::IsNullOrEmpty($uri.Query) -or
      -not [string]::IsNullOrEmpty($uri.Fragment)) {
    throw "$Name must not contain userinfo, a query, or a fragment"
  }
  if ($uri.Host.Contains(":") -or $uri.Host -cne $uri.Host.ToLowerInvariant()) {
    throw "$Name has a non-canonical host: $Value"
  }

  $path = if ($uri.AbsolutePath -eq "/") { "" } else { $uri.AbsolutePath.TrimEnd("/") }
  if ($path -notmatch '^(/[A-Za-z0-9._~/-]+)?$' -or
      $path.Contains("//") -or
      $path -match '(^|/)\.\.?(/|$)') {
    throw "$Name has a non-canonical path: $Value"
  }
  if ($RequireOrigin -and -not [string]::IsNullOrEmpty($path)) {
    throw "$Name must be an origin without a path: $Value"
  }

  $authority = if ($uri.IsDefaultPort) {
    "$($uri.Scheme)://$($uri.Host)"
  } else {
    "$($uri.Scheme)://$($uri.Host):$($uri.Port)"
  }
  $canonical = "$authority$path"
  if ($Value -cne $canonical) {
    throw "$Name is not canonical: $Value"
  }
  return $canonical
}

function Test-SemVer {
  param([string]$Value)
  return $Value -match '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
}

function Get-ReleaseArchitecture {
  $processArchitecture = [string]$env:PROCESSOR_ARCHITECTURE
  switch ($processArchitecture.ToUpperInvariant()) {
    "AMD64" { return "x86_64" }
    # The release matrix ships an x86_64 Windows binary. Windows on ARM64
    # runs it through the operating system's x64 compatibility layer.
    "ARM64" { return "x86_64" }
    "X86" {
      $nativeArchitecture = [string]$env:PROCESSOR_ARCHITEW6432
      switch ($nativeArchitecture.ToUpperInvariant()) {
        "AMD64" { return "x86_64" }
        "ARM64" { return "x86_64" }
        default { throw "32-bit Windows is not supported; install Chat2DB CLI on 64-bit Windows" }
      }
    }
    default { throw "unsupported Windows architecture: $processArchitecture" }
  }
}

function Assert-CanonicalExactUrl {
  param(
    [Parameter(Mandatory = $true)][string]$Actual,
    [Parameter(Mandatory = $true)][string]$Expected,
    [Parameter(Mandatory = $true)][string]$Name
  )

  $uri = $null
  if (-not [System.Uri]::TryCreate($Actual, [System.UriKind]::Absolute, [ref]$uri) -or
      -not [string]::IsNullOrEmpty($uri.UserInfo) -or
      -not [string]::IsNullOrEmpty($uri.Query) -or
      -not [string]::IsNullOrEmpty($uri.Fragment) -or
      $uri.AbsoluteUri -cne $Actual -or
      $Actual -cne $Expected) {
    throw "$Name is not the expected canonical URL: $Actual (expected $Expected)"
  }
}

function Get-FileSha256 {
  param([Parameter(Mandatory = $true)][string]$Path)

  $stream = [System.IO.File]::OpenRead($Path)
  try {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
      $hash = $sha256.ComputeHash($stream)
      return ([System.BitConverter]::ToString($hash)).Replace("-", "").ToLowerInvariant()
    } finally {
      $sha256.Dispose()
    }
  } finally {
    $stream.Dispose()
  }
}

if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = "latest"
}
if ($Version -ne "latest" -and -not (Test-SemVer $Version)) {
  throw "invalid release version: $Version"
}
if ([string]::IsNullOrWhiteSpace($ManifestBase)) {
  $ManifestBase = "https://github.com/OtterMind/Chat2DB-CLI/releases"
}
if ([string]::IsNullOrWhiteSpace($CdnBase)) {
  $CdnBase = "https://cdn.chat2db-ai.com"
}
$ManifestBase = Get-ValidatedUrlBase -Value $ManifestBase -Name "CHAT2DB_CLI_MANIFEST_BASE"
$CdnBase = Get-ValidatedUrlBase -Value $CdnBase -Name "CHAT2DB_CLI_CDN_BASE" -RequireOrigin

if ([string]::IsNullOrWhiteSpace($BinDir)) {
  $localAppData = $env:LOCALAPPDATA
  if ([string]::IsNullOrWhiteSpace($localAppData)) {
    $localAppData = Join-Path $env:USERPROFILE "AppData\Local"
  }
  $BinDir = Join-Path $localAppData "Programs\chat2db-cli"
}

$arch = Get-ReleaseArchitecture

$manifestName = "manifest-cli-windows-$arch.json"
$manifestUrl = if ($Version -eq "latest") {
  "$ManifestBase/latest/download/$manifestName"
} else {
  "$ManifestBase/download/v$Version/$manifestName"
}
Assert-CanonicalExactUrl -Actual $manifestUrl -Expected $manifestUrl -Name "manifest URL"

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("chat2db-cli-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null

try {
  $manifestPath = Join-Path $stage "manifest.json"
  Invoke-WebRequest -Uri $manifestUrl -OutFile $manifestPath -UseBasicParsing -MaximumRedirection 5
  $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  $manifestVersion = [string]$manifest.version
  if (-not (Test-SemVer $manifestVersion) -or
      ($Version -ne "latest" -and $manifestVersion -cne $Version)) {
    throw "manifest version '$manifestVersion' does not match requested release '$Version'"
  }
  if ($null -eq $manifest.artifact -or [string]$manifest.artifact.artifact_type -cne "zip") {
    throw "Windows CLI artifact_type must be zip"
  }

  $expectedSha256 = [string]$manifest.artifact.sha256
  if ($expectedSha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw "manifest sha256 must contain 64 lowercase hexadecimal characters"
  }
  $archiveName = "chat2db-$manifestVersion-windows-$arch.zip"
  $expectedArtifactUrl = "$CdnBase/download/chat2db-cli/$manifestVersion/windows/$arch/$archiveName"
  $artifactUrl = [string]$manifest.artifact.url
  Assert-CanonicalExactUrl -Actual $artifactUrl -Expected $expectedArtifactUrl -Name "artifact URL"

  $archivePath = Join-Path $stage $archiveName
  Invoke-WebRequest -Uri $artifactUrl -OutFile $archivePath -UseBasicParsing -MaximumRedirection 5
  $actualSha256 = Get-FileSha256 -Path $archivePath
  if ($actualSha256 -cne $expectedSha256) {
    throw "checksum mismatch: expected $expectedSha256, got $actualSha256"
  }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $extractDir = Join-Path $stage "extract"
  New-Item -ItemType Directory -Path $extractDir | Out-Null
  $source = Join-Path $extractDir "chat2db.exe"
  $archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
  try {
    $expectedEntries = @("LICENSE", "THIRD_PARTY_LICENSES.txt", "chat2db.exe")
    $actualEntries = @($archive.Entries | ForEach-Object { $_.FullName } | Sort-Object -CaseSensitive)
    $entryDifference = @(Compare-Object $expectedEntries $actualEntries -CaseSensitive)
    if ($archive.Entries.Count -ne 3 -or $entryDifference.Count -ne 0) {
      throw "Windows CLI archive must contain only chat2db.exe, LICENSE, and THIRD_PARTY_LICENSES.txt at the root"
    }
    foreach ($archiveEntry in $archive.Entries) {
      if ($archiveEntry.Name -cne $archiveEntry.FullName -or $archiveEntry.Length -le 0) {
        throw "Windows CLI archive entries must be non-empty regular files at the root"
      }
    }
    $entry = @($archive.Entries | Where-Object { $_.FullName -ceq "chat2db.exe" })[0]
    $inputStream = $entry.Open()
    $outputStream = [System.IO.File]::Open(
      $source,
      [System.IO.FileMode]::CreateNew,
      [System.IO.FileAccess]::Write,
      [System.IO.FileShare]::None
    )
    try {
      $inputStream.CopyTo($outputStream)
    } finally {
      $outputStream.Dispose()
      $inputStream.Dispose()
    }
  } finally {
    $archive.Dispose()
  }
  if ((Get-Item -LiteralPath $source).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
    throw "Windows CLI archive entry must not be a reparse point"
  }

  New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
  $target = Join-Path $BinDir "chat2db.exe"
  if (Test-Path -LiteralPath $target -PathType Container) {
    throw "chat2db target is a directory and cannot be replaced: $target"
  }
  if ((Test-Path -LiteralPath $target) -and -not $Force) {
    throw "chat2db already exists at $target; rerun with -Force to replace"
  }

  $pending = Join-Path $BinDir (".chat2db.pending." + [System.Guid]::NewGuid().ToString("N") + ".exe")
  $backup = Join-Path $BinDir (".chat2db.backup." + [System.Guid]::NewGuid().ToString("N") + ".exe")
  $previousMoved = $false
  try {
    Copy-Item -LiteralPath $source -Destination $pending
    $candidateOutput = @(& $pending version 2>&1)
    if ($LASTEXITCODE -ne 0) {
      throw "downloaded CLI failed its pre-installation version check"
    }

    if (Test-Path -LiteralPath $target) {
      $previousMoved = $true
      Move-Item -LiteralPath $target -Destination $backup
    }
    Move-Item -LiteralPath $pending -Destination $target

    if ($previousMoved) {
      Remove-Item -LiteralPath $backup -Force
      $previousMoved = $false
    }
  } catch {
    $installError = $_
    if ($previousMoved -and (Test-Path -LiteralPath $backup)) {
      try {
        Remove-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
        Move-Item -LiteralPath $backup -Destination $target
        $previousMoved = $false
      } catch {
        throw "CLI installation failed and the previous version could not be restored; backup preserved at ${backup}: $installError"
      }
    }
    throw $installError
  } finally {
    Remove-Item -LiteralPath $pending -Force -ErrorAction SilentlyContinue
    if (-not $previousMoved) {
      Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
  }

  if (-not $NoModifyPath -and $env:CHAT2DB_CLI_NO_MODIFY_PATH -ne "1") {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if (-not [string]::IsNullOrWhiteSpace($userPath)) {
      $entries = $userPath -split ";"
    }
    if (-not ($entries | Where-Object { $_ -ieq $BinDir })) {
      $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $BinDir } else { "$userPath;$BinDir" }
      [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
      Write-Output "path: added $BinDir to the user PATH"
    } else {
      Write-Output "path: already configured"
    }
  } else {
    Write-Output "path: skipped; add $BinDir to PATH manually"
  }

  Write-Output "installed: $target"
  $candidateOutput | Write-Output
} finally {
  Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
