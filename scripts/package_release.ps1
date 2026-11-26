param([string]$Version = "", [switch]$SkipBuild)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    $metadata = cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Cannot read Cargo package metadata" }
    $package = @($metadata.packages | Where-Object name -eq "vrclt")
    if ($package.Count -ne 1) { throw "Expected one vrclt package" }
    $actual = $package[0].version
    if ($Version -and $Version.TrimStart('v') -ne $actual) { throw "Expected version $actual" }
    if (-not $SkipBuild) { & "$PSScriptRoot/build_native.ps1" }
    $built = Get-Item -LiteralPath (Join-Path $metadata.target_directory "release/vrclt.exe")
    if ($built.VersionInfo.ProductVersion -ne $actual) { throw "Executable does not match v$actual" }
    $out = [IO.Directory]::CreateDirectory((Join-Path $root "release")).FullName
    $stem = "vrclt-v$actual-windows-x64"
    $exe = Join-Path $out "$stem.exe"
    $zip = Join-Path $out "$stem.zip"
    Copy-Item -LiteralPath $built.FullName -Destination $exe -Force
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $tmp = Join-Path $out ("$stem-" + [guid]::NewGuid() + ".tmp")
    try {
        $archive = [IO.Compression.ZipFile]::Open($tmp, 'Create')
        try {
            $files = [ordered]@{ "vrclt/vrclt.exe" = $exe; "vrclt/LICENSE.txt" = "$root/LICENSE" }
            foreach ($file in (Get-ChildItem -File -Path licenses/*.txt,assets/fonts/LICENSE-*.txt)) {
                $files["vrclt/licenses/" + $file.Name] = $file.FullName
            }
            foreach ($entry in $files.GetEnumerator()) {
                [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $entry.Value, $entry.Key, 'Optimal')
            }
        } finally { $archive.Dispose() }
        $archive = [IO.Compression.ZipFile]::OpenRead($tmp)
        try {
            $stream = $archive.GetEntry("vrclt/vrclt.exe").Open()
            try { $inner = (Get-FileHash -InputStream $stream).Hash } finally { $stream.Dispose() }
            if ($inner -ne (Get-FileHash -LiteralPath $exe).Hash) { throw "ZIP executable differs from the standalone build" }
        } finally { $archive.Dispose() }
        Move-Item -LiteralPath $tmp -Destination $zip -Force
    } finally { if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp } }
    foreach ($file in @($exe,$zip)) {
        $sum = (Get-FileHash -LiteralPath $file).Hash.ToLowerInvariant()
        Set-Content -LiteralPath "$file.sha256" -Encoding ascii -Value "$sum  $([IO.Path]::GetFileName($file))"
    }
} finally { Pop-Location }
