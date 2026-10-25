<#
.SYNOPSIS
Builds OZBEAT as an MSIX package for the Microsoft Store.

.DESCRIPTION
Builds the release exe, renders the tile images, fills in the manifest and
packs everything into dist\OZBEAT-<version>-x64.msix.

For the Store, take -IdentityName, -Publisher and -PublisherDisplayName from
Partner Center (Product identity) and upload the unsigned .msix: the Store
signs it. To install it on this PC first, add -TestSign.

.EXAMPLE
.\packaging\msix\build.ps1 -TestSign

.EXAMPLE
.\packaging\msix\build.ps1 -IdentityName 12345Ondra.OZBEAT -Publisher "CN=ABCD1234-..." -PublisherDisplayName Ondra
#>
param(
    [string]$IdentityName = "oondraa.OZBEAT",
    [string]$Publisher = "CN=oondraa",
    [string]$PublisherDisplayName = "Ondra",
    # Signs with a self-signed certificate so the package installs locally.
    [switch]$TestSign,
    # Packs the exe already in target\release instead of building it.
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$here = $PSScriptRoot

# The Store wants four parts with the last one 0: 0.1.12 -> 0.1.12.0.
$cargo = Get-Content (Join-Path $root "app\Cargo.toml") -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') { throw "no version in app\Cargo.toml" }
$version = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"

$kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$sdk = Get-ChildItem $kits -Directory -Filter "10.*" |
    Where-Object { Test-Path (Join-Path $_.FullName "x64\makeappx.exe") } |
    Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $sdk) { throw "makeappx.exe not found: install the Windows SDK" }
$bin = Join-Path $sdk.FullName "x64"

if (-not $SkipBuild) {
    cargo build --release -p musicvisual --manifest-path (Join-Path $root "Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$stage = Join-Path $root "target\msix\stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root "target\release\musicvisual.exe") (Join-Path $stage "OZBEAT.exe")
Copy-Item (Join-Path $root "LICENSE") $stage
Copy-Item (Join-Path $root "THIRD_PARTY_LICENSES.md") $stage

python (Join-Path $here "make_assets.py") (Join-Path $stage "Assets")
if ($LASTEXITCODE -ne 0) { throw "make_assets.py failed" }

$manifest = (Get-Content (Join-Path $here "AppxManifest.xml") -Raw).
    Replace("{{IdentityName}}", $IdentityName).
    Replace("{{Publisher}}", [Security.SecurityElement]::Escape($Publisher)).
    Replace("{{PublisherDisplayName}}", [Security.SecurityElement]::Escape($PublisherDisplayName)).
    Replace("{{Version}}", $version)
[IO.File]::WriteAllText((Join-Path $stage "AppxManifest.xml"), $manifest)

# resources.pri lets Windows pick the right image scale and size.
$priConfig = Join-Path $root "target\msix\priconfig.xml"
& "$bin\makepri.exe" createconfig /cf $priConfig /dq en-US /o | Out-Null
& "$bin\makepri.exe" new /pr $stage /cf $priConfig /mn (Join-Path $stage "AppxManifest.xml") /of (Join-Path $stage "resources.pri") /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makepri failed" }

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force $dist | Out-Null
$msix = Join-Path $dist "OZBEAT-$version-x64.msix"
& "$bin\makeappx.exe" pack /o /h SHA256 /d $stage /p $msix | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

if ($TestSign) {
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $Publisher -and $_.NotAfter -gt (Get-Date) } |
        Select-Object -First 1
    if (-not $cert) {
        $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher `
            -KeyUsage DigitalSignature -FriendlyName "OZBEAT test signing" `
            -CertStoreLocation Cert:\CurrentUser\My `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    }
    & "$bin\signtool.exe" sign /fd SHA256 /sha1 $cert.Thumbprint $msix | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "signtool failed" }
    $cer = Join-Path $root "target\msix\OZBEAT-test.cer"
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    Write-Host "Signed for testing. To install it, trust the certificate once (admin PowerShell):"
    Write-Host "  Import-Certificate -FilePath `"$cer`" -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
    Write-Host "then double-click the .msix."
}

Write-Host "Package: $msix"
