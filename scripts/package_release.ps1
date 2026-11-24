param(
    [Parameter(Mandatory = $false)]
    [string]$Version = "",

    [Parameter(Mandatory = $false)]
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir
$VersionSource = Get-Content -LiteralPath (Join-Path $Root "vrclt\__init__.py") -Raw
if ($VersionSource -notmatch '__version__\s*=\s*"([^"]+)"') {
    throw "Could not read the application version."
}
$AppVersion = $Matches[1]
$RequestedVersion = if ([string]::IsNullOrWhiteSpace($Version)) {
    $AppVersion
} else {
    $Version.TrimStart('v')
}
if ($RequestedVersion -ne $AppVersion) {
    throw "Release version $RequestedVersion does not match application version $AppVersion."
}
$VersionName = "v$AppVersion"
$Python = Join-Path $Root ".venv\Scripts\python.exe"
$ReleaseDir = Join-Path $Root "release"
$DistExe = Join-Path $Root "dist\vrclt.exe"
$DistFolder = Join-Path $Root "dist\vrclt"
$DistFolderExe = Join-Path $DistFolder "vrclt.exe"
$ReleaseExe = Join-Path $ReleaseDir "vrclt-$VersionName-windows-x64.exe"
$ReleaseZip = Join-Path $ReleaseDir "vrclt-$VersionName-windows-x64.zip"

if (-not $SkipBuild) {
    if (-not (Test-Path $Python)) {
        throw "Virtual environment not found: $Python"
    }

    Push-Location -LiteralPath $Root
    try {
        & $Python -m PyInstaller (Join-Path $Root "vrclt.spec") --noconfirm
        if ($LASTEXITCODE -ne 0) {
            throw "PyInstaller failed. Existing executables will not be packaged."
        }
    } finally {
        Pop-Location
    }
}

# the spec builds the single file and the folder from one analysis
foreach ($Built in @($DistExe, $DistFolderExe)) {
    if (-not (Test-Path $Built)) {
        throw "Build output not found. Expected: $Built"
    }
    # -SkipBuild must not relabel an executable left over from an older version.
    & $Python -c 'import sys; from PyInstaller.archive.readers import CArchiveReader; archive = CArchiveReader(sys.argv[1]).open_embedded_archive("PYZ.pyz"); code = archive.extract("vrclt"); assert sys.argv[2] in code.co_consts, "Packaged application version does not match release version"' $Built $AppVersion
    if ($LASTEXITCODE -ne 0) {
        throw "$Built does not match v$AppVersion. Rebuild before packaging."
    }
}

New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
Copy-Item -LiteralPath $DistExe -Destination $ReleaseExe -Force

# The zip holds one top-level "vrclt" folder, so a new version extracted over
# the old one keeps the path SteamVR auto-start points at.
if (Test-Path $ReleaseZip) {
    Remove-Item -LiteralPath $ReleaseZip -Force
}
& $Python -c 'import shutil, sys; shutil.make_archive(sys.argv[1][:-4], "zip", root_dir=sys.argv[2], base_dir="vrclt")' $ReleaseZip (Split-Path -Parent $DistFolder)
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $ReleaseZip)) {
    throw "Could not create $ReleaseZip"
}

foreach ($Artifact in @($ReleaseExe, $ReleaseZip)) {
    $Hash = Get-FileHash -LiteralPath $Artifact -Algorithm SHA256
    Set-Content -LiteralPath "$Artifact.sha256" -Encoding ascii -Value "$($Hash.Hash)  $(Split-Path $Artifact -Leaf)"
    Write-Host "Release artifact: $Artifact"
    Write-Host "SHA256 checksum:  $Artifact.sha256"
}
