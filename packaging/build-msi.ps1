# Builds the per-user MSI installer from target\release\images.exe with WiX Toolset v5:
#
#   dotnet tool install --global wix --version 5.0.2
#   cargo build --release
#   .\packaging\build-msi.ps1                                # dist\images-<version>-x64.msi
param(
    [string]$Exe = "target\release\images.exe",
    [string]$OutDir = "dist"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    $extensions = (Get-Content packaging\extensions.txt | ForEach-Object { $_.Trim() } |
        Where-Object { $_.StartsWith(".") }) -join ";"

    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $out = Join-Path $OutDir "images-$version-x64.msi"
    wix build packaging\wix\Images.wxs -arch x64 -o $out `
        -d "Version=$version" `
        -d "Exe=$((Resolve-Path $Exe).Path)" `
        -d "Icon=$((Resolve-Path assets\images.ico).Path)" `
        -d "Extensions=$extensions"
    if ($LASTEXITCODE) { throw "wix build failed" }
    Remove-Item (Join-Path $OutDir "*.wixpdb") -ErrorAction SilentlyContinue
    Write-Host "Built $out"
}
finally {
    Pop-Location
}
