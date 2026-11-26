param([switch]$Test)
$ErrorActionPreference = "Stop"
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $vswhere = Join-Path ([Environment]::GetFolderPath("ProgramFilesX86")) "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $vswhere) {
        $vs = & $vswhere -latest -products '*' -property installationPath
        if ($vs) {
            $cmakeDir = Join-Path $vs "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
            $clangDir = Join-Path $vs "VC\Tools\Llvm\x64\bin"
            if (Test-Path "$cmakeDir\cmake.exe") { $env:Path = "$cmakeDir;$env:Path" }
            if (!$env:LIBCLANG_PATH -and (Test-Path "$clangDir\libclang.dll")) { $env:LIBCLANG_PATH = $clangDir }
        }
    }
    $env:CMAKE_POLICY_VERSION_MINIMUM = "3.5"
    if ($Test) {
        cargo test --release --locked
        if ($LASTEXITCODE) { throw "Rust tests failed" }
    }
    cargo build --release --locked
    if ($LASTEXITCODE) { throw "Rust build failed" }
    if ($Test) {
        $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
        & "$PSScriptRoot/../tests/native_update.ps1" -Executable (Join-Path $metadata.target_directory "release/vrclt.exe")
    }
} finally {
    Pop-Location
}
