<#
.SYNOPSIS
  Build the Windows release artifacts: a portable ZIP and an MSI, then hash
  (and sign) them.

.DESCRIPTION
  Produces, in dist\ :
    pixiv-manager-<version>-x86_64-windows.zip   portable, no install needed
    PixivManager-<version>-x86_64.msi            per-user installer (WiX)
    SHA256SUMS.txt                               hashes for every artifact

  Signing is optional and driven by secrets, never by the repository:
    * WINDOWS_CERT_PFX_BASE64 + WINDOWS_CERT_PASSWORD  -> signtool with a PFX
  When the secrets are absent the artifacts are still built and hashed, and the
  script says so explicitly, so forked PRs and unsigned local builds work.

  Deliberately avoids: UPX/packers, self-extracting archives, bootstrap
  downloaders, runtime extraction - all of which wreck antivirus reputation
  for a signed desktop app.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$BinDir = "target\release",
    [string]$DistDir = "dist"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Step($message) { Write-Host "==> $message" }

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$zipName = "pixiv-manager-$Version-x86_64-windows"
$zipPath = Join-Path $DistDir "$zipName.zip"
$msiPath = Join-Path $DistDir "PixivManager-$Version-x86_64.msi"

New-Item -ItemType Directory -Force $DistDir | Out-Null

# -- 1. Portable ZIP -----------------------------------------------------
Step "Staging portable ZIP contents"
$stage = Join-Path $DistDir $zipName
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($exe in @("pixiv-manager.exe", "pixiv-manager-cli.exe")) {
    $src = Join-Path $BinDir $exe
    if (-not (Test-Path $src)) { throw "missing $src - run cargo build --release first" }
    Copy-Item $src $stage
}
Copy-Item README.md $stage
if (Test-Path LICENSE) { Copy-Item LICENSE $stage }

Step "Writing $zipPath"
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath -CompressionLevel Optimal
Remove-Item -Recurse -Force $stage

# -- 2. MSI (WiX) --------------------------------------------------------
Step "Building MSI with WiX"
$wix = Get-Command wix -ErrorAction SilentlyContinue
if ($null -eq $wix) {
    $candidate = Join-Path $env:USERPROFILE ".dotnet\tools\wix.exe"
    if (Test-Path $candidate) { $wix = @{ Source = $candidate } }
}
if ($null -eq $wix) {
    Write-Warning "WiX (dotnet tool 'wix') not found - skipping MSI."
    Write-Warning "Install with: dotnet tool install --global wix"
}
else {
    & $wix.Source build packaging\windows\PixivManager.wxs `
        -arch x64 `
        -d Version=$Version `
        -d BinDir=$BinDir `
        -o $msiPath
    if ($LASTEXITCODE -ne 0) { throw "wix build failed with $LASTEXITCODE" }
}

# -- 3. Authenticode signing ---------------------------------------------
function Find-SdkTool([string]$name) {
    $command = Get-Command $name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $roots = @("${env:ProgramFiles(x86)}\Windows Kits\10\bin", "$env:ProgramFiles\Windows Kits\10\bin")
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        $found = Get-ChildItem $root -Recurse -Filter $name -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1
        if ($found) { return $found.FullName }
    }
    return $null
}

$signtool = Find-SdkTool "signtool.exe"
$pfxBase64 = $env:WINDOWS_CERT_PFX_BASE64
$pfxPassword = $env:WINDOWS_CERT_PASSWORD
$signTargets = @($zipPath, $msiPath) | Where-Object { Test-Path $_ }

if ([string]::IsNullOrWhiteSpace($pfxBase64) -or [string]::IsNullOrWhiteSpace($pfxPassword)) {
    Write-Warning "No signing certificate in the environment - artifacts are UNSIGNED."
    Write-Warning "Set WINDOWS_CERT_PFX_BASE64 and WINDOWS_CERT_PASSWORD (base64 PFX) to sign."
}
elseif ($null -eq $signtool) {
    Write-Warning "signtool.exe not found - artifacts are UNSIGNED."
}
else {
    Step "Signing artifacts (Authenticode, SHA-256, timestamped)"
    $pfx = Join-Path ([System.IO.Path]::GetTempPath()) "pixiv-manager-signing.pfx"
    try {
        [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($pfxBase64))
        $exeSignTargets = @((Join-Path $BinDir "pixiv-manager.exe"), (Join-Path $BinDir "pixiv-manager-cli.exe"))
        foreach ($target in $exeSignTargets + @($msiPath)) {
            if (-not (Test-Path $target)) { continue }
            & $signtool sign /fd SHA256 /f $pfx /p $pfxPassword `
                /tr http://timestamp.digicert.com /td SHA256 $target
            if ($LASTEXITCODE -ne 0) { throw "signing failed for $target" }
            & $signtool verify /pa /v $target
            if ($LASTEXITCODE -ne 0) { throw "signature verification failed for $target" }
        }
        # The ZIP was staged from the *signed* binaries only if it is rebuilt
        # after signing; rebuild it so the portable package ships signed exes.
        Step "Re-creating ZIP from signed executables"
        $stage = Join-Path $DistDir $zipName
        New-Item -ItemType Directory -Force $stage | Out-Null
        Copy-Item $exeSignTargets[0], $exeSignTargets[1], README.md $stage
        if (Test-Path LICENSE) { Copy-Item LICENSE $stage }
        Remove-Item -Force $zipPath
        Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath -CompressionLevel Optimal
        Remove-Item -Recurse -Force $stage
        & $signtool verify /pa $zipPath | Out-Null
    }
    finally {
        Remove-Item -Force $pfx -ErrorAction SilentlyContinue
    }
}

# -- 4. Hashes -----------------------------------------------------------
Step "Generating SHA256SUMS.txt"
$lines = @()
foreach ($artifact in ($signTargets + (Get-ChildItem $DistDir -Filter "*.msi" -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })) | Select-Object -Unique) {
    if (-not (Test-Path $artifact)) { continue }
    $hash = (Get-FileHash -Algorithm SHA256 -Path $artifact).Hash.ToLower()
    $lines += "$hash  $(Split-Path -Leaf $artifact)"
}
$lines | Set-Content -Encoding ascii (Join-Path $DistDir "SHA256SUMS.txt")

Step "Artifacts in $DistDir"
Get-ChildItem $DistDir | Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Host
