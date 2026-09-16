# Install the skilld CLI without Node.js.
#
#   irm https://github.com/skilld-dev/skilld/releases/latest/download/install.ps1 | iex
#
# Environment:
#   SKILLD_INSTALL_DIR  Install directory. Default: %LOCALAPPDATA%\skilld\bin
#   SKILLD_VERSION      Exact version to install, for example 3.1.0. Default: latest
#
# PowerShell cannot check Ed25519 signatures, so this script trusts the HTTPS
# download and the SHA-256 digest. The installed CLI verifies the release
# signature before every upgrade.
$ErrorActionPreference = 'Stop'

$repository = 'https://github.com/skilld-dev/skilld'
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
  'AMD64' { 'x64' }
  'ARM64' { 'arm64' }
  default { throw "skilld has no build for $env:PROCESSOR_ARCHITECTURE." }
}
$asset = "skilld-cli-win32-$arch-msvc.exe"
$release = if ($env:SKILLD_VERSION) { "$repository/releases/download/v$env:SKILLD_VERSION" } else { "$repository/releases/latest/download" }
$installDir = if ($env:SKILLD_INSTALL_DIR) { $env:SKILLD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'skilld\bin' }

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("skilld-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
  $manifestPath = Join-Path $work 'skilld-release.txt'
  Invoke-WebRequest -UseBasicParsing -Uri "$release/skilld-release.txt" -OutFile $manifestPath
  $lines = Get-Content -LiteralPath $manifestPath
  if ($lines[0] -ne 'skilld-release-v1' -or $lines[1] -notmatch '^version ([0-9A-Za-z.-]+)$') {
    throw 'The release manifest is invalid.'
  }
  $version = $Matches[1]
  $entry = $lines | Where-Object { ($_ -split '  ', 2)[1] -eq $asset } | Select-Object -First 1
  if (-not $entry) { throw "The release has no $asset build." }
  $expected = ($entry -split '  ', 2)[0]

  $binary = Join-Path $work 'skilld.exe'
  Invoke-WebRequest -UseBasicParsing -Uri "$release/$asset" -OutFile $binary
  if ((Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant() -ne $expected) {
    throw 'The downloaded binary does not match its digest. Nothing was installed.'
  }
  if ((& $binary --version) -ne "skilld $version") {
    throw 'The downloaded binary reports the wrong version.'
  }

  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  $target = Join-Path $installDir 'skilld.exe'
  if (Test-Path -LiteralPath $target) {
    Move-Item -Force -LiteralPath $target -Destination "$target.old"
  }
  Move-Item -LiteralPath $binary -Destination $target
  Set-Content -LiteralPath (Join-Path $installDir 'skilld-install.json') -Value '{"channel":"standalone"}'

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($userPath -split ';') -contains $installDir)) {
    [Environment]::SetEnvironmentVariable('Path', "$installDir;$userPath", 'User')
    Write-Host "Added $installDir to your PATH. Open a new terminal to use skilld."
  }
  Write-Host "Installed skilld $version to $target."
}
finally {
  Remove-Item -Recurse -Force -LiteralPath $work
}
