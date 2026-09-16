$ErrorActionPreference = "Stop"

$repo = "kossnocorp/vdm"
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture

if ($architecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
  throw "Unsupported Windows architecture: $architecture"
}

$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ "User-Agent" = "vdm-installer" }
$tag = $release.tag_name
$assetName = "vdm-$tag-x86_64-pc-windows-msvc.exe"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1

if (-not $asset) {
  throw "Release asset not found: $assetName"
}

$installDir = if ($env:VDM_INSTALL_DIR) {
  $env:VDM_INSTALL_DIR
} else {
  Join-Path $HOME ".local\bin"
}
$destination = Join-Path $installDir "vdm.exe"
$tempFile = [System.IO.Path]::GetTempFileName()

try {
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tempFile
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  Move-Item -Force $tempFile $destination
} finally {
  if (Test-Path $tempFile) {
    Remove-Item -Force $tempFile
  }
}

Write-Host "Installed Vdm to $destination"
$pathEntries = $env:PATH -split [System.IO.Path]::PathSeparator
if ($installDir -notin $pathEntries) {
  Write-Host "Add $installDir to PATH to run vdm."
}
