# Builds the MSIX package (Microsoft Store / sideloading) from target\release\images.exe.
# Needs the Windows SDK (makeappx.exe, makepri.exe). The package is unsigned: the Store signs it on
# submission; to sideload it yourself, sign it with signtool and a certificate matching -Publisher.
#
#   cargo build --release
#   .\packaging\build-msix.ps1                               # dist\images-<version>-x64.msix
#   .\packaging\build-msix.ps1 -IdentityName 12345Sid.Images -Publisher "CN=ABCD-..." -PublisherDisplayName "Sid"
param(
    # Store builds: use the Package/Identity values from Partner Center > Product identity.
    [string]$IdentityName = "sidx1024.Images",
    [string]$Publisher = "CN=sidx1024",
    [string]$PublisherDisplayName = "sidx1024",
    # The reserved app name.
    [string]$DisplayName = "Images",
    # Defaults to Cargo.toml's version plus ".0". The Store needs a first part of at least 1.
    [string]$Version,
    [string]$Exe = "target\release\images.exe",
    [string]$OutDir = "dist"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $cargoVersion = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    # MSIX versions have four parts (each 0-65535); the Store requires the last one to be 0.
    $msixVersion = if ($Version) { $Version } else { "$cargoVersion.0" }
    if ($msixVersion -notmatch '^\d{1,5}\.\d{1,5}\.\d{1,5}\.0$' -or ($msixVersion.Split('.') | Where-Object { [int]$_ -gt 65535 })) {
        throw "MSIX version $msixVersion must be a.b.c.0 with parts up to 65535"
    }
    if ($msixVersion.StartsWith("0.")) { Write-Warning "MSIX version $msixVersion starts with 0, which the Store rejects" }

    # Newest Windows SDK that has the packaging tools.
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $sdk = Get-ChildItem $kits -Directory | Where-Object { Test-Path "$($_.FullName)\x64\makeappx.exe" } |
        Sort-Object { [version]$_.Name } | Select-Object -Last 1
    if (-not $sdk) { throw "Windows SDK packaging tools (makeappx.exe) not found under $kits" }
    $makeappx = "$($sdk.FullName)\x64\makeappx.exe"
    $makepri = "$($sdk.FullName)\x64\makepri.exe"

    $stage = Join-Path $OutDir "msix-stage"
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
    Copy-Item $Exe "$stage\images.exe"
    Copy-Item "packaging\msix\Assets\*.png" "$stage\Assets"

    $fileTypes = Get-Content packaging\extensions.txt | ForEach-Object { $_.Trim() } |
        Where-Object { $_.StartsWith(".") } |
        ForEach-Object { "              <uap:FileType>$_</uap:FileType>" }
    $esc = { param($s) [Security.SecurityElement]::Escape($s) }
    $manifest = Get-Content packaging\msix\AppxManifest.xml -Raw
    $manifest = $manifest.Replace("{{IdentityName}}", (& $esc $IdentityName))
    $manifest = $manifest.Replace("{{Publisher}}", (& $esc $Publisher))
    $manifest = $manifest.Replace("{{PublisherDisplayName}}", (& $esc $PublisherDisplayName))
    $manifest = $manifest.Replace("{{DisplayName}}", (& $esc $DisplayName))
    $manifest = $manifest.Replace("{{Version}}", $msixVersion)
    $manifest = $manifest.Replace("{{FileTypes}}", ($fileTypes -join "`n"))
    Set-Content -Encoding utf8 "$stage\AppxManifest.xml" $manifest

    # resources.pri lets Windows pick the scale/targetsize logo variants.
    $priConfig = Join-Path $OutDir "priconfig.xml"
    & $makepri createconfig /cf $priConfig /dq en-US /pv 10.0.0 /o | Out-Null
    if ($LASTEXITCODE) { throw "makepri createconfig failed" }
    # One flat package (no resource packs), so keep every scale in the main resources.pri.
    $cfg = [xml](Get-Content $priConfig)
    $cfg.SelectNodes("//packaging") | ForEach-Object { [void]$_.ParentNode.RemoveChild($_) }
    $cfg.Save((Resolve-Path $priConfig))
    & $makepri new /pr $stage /cf $priConfig /mn "$stage\AppxManifest.xml" /of "$stage\resources.pri" /o | Out-Null
    if ($LASTEXITCODE) { throw "makepri new failed" }

    $out = Join-Path $OutDir "images-$cargoVersion-x64.msix"
    & $makeappx pack /d $stage /p $out /o
    if ($LASTEXITCODE) { throw "makeappx pack failed" }
    Remove-Item $stage, $priConfig -Recurse -Force
    Write-Host "Built $out"
}
finally {
    Pop-Location
}
