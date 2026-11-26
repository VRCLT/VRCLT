param([Parameter(Mandatory)][string]$Executable)
$ErrorActionPreference = "Stop"
$built = (Get-Item -LiteralPath $Executable).FullName
$expected = (Get-FileHash -LiteralPath $built).Hash.ToLowerInvariant()
$root = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testDir = [IO.Directory]::CreateDirectory((Join-Path $root ("vrclt-update-test-" + [guid]::NewGuid()))).FullName
try {
    $fixture = Join-Path $testDir "fixture.cs"
    @'
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
[assembly: AssemblyProduct("VRCLT")]
[assembly: AssemblyInformationalVersion("0.1.0")]
class EarlierVersion {
    static int Main(string[] args) {
        var start = new ProcessStartInfo(args[0], "--finish-update " + args[1] + " --help");
        start.UseShellExecute = false;
        start.RedirectStandardInput = true;
        start.RedirectStandardOutput = true;
        start.EnvironmentVariables["LOCALAPPDATA"] = args[3];
        using (var child = Process.Start(start)) {
            File.WriteAllText(Path.Combine(args[3], "helper.pid"), child.Id.ToString());
            string reply = child.StandardOutput.ReadLine();
            if (args[2] == "corrupt")
                return reply != null && reply.StartsWith("ERROR ") ? 0 : 2;
            if (reply != "READY") { Console.Error.WriteLine(reply); return 3; }
            if (args[2] == "tampered") {
                string staged = Directory.GetFiles(args[3], ".tmp*")[0];
                using (var file = new FileStream(staged, FileMode.Open, FileAccess.Write, FileShare.ReadWrite | FileShare.Delete))
                    file.WriteByte(0);
            }
            if (args[2] != "cancel") child.StandardInput.WriteLine(args[2] == "restart" ? "restart" : "install");
            child.StandardInput.Close();
            return 0;
        }
    }
}
'@ | Set-Content -LiteralPath $fixture
    $starter = Join-Path $testDir "starter.exe"
    & "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo "/out:$starter" $fixture
    if ($LASTEXITCODE) { throw "Could not build the older-version fixture" }
    $oldHash = (Get-FileHash -LiteralPath $starter).Hash.ToLowerInvariant()
    foreach ($mode in @("cancel", "corrupt", "tampered", "locked", "install", "restart")) {
        $dir = [IO.Directory]::CreateDirectory((Join-Path $testDir "$mode folder")).FullName
        [void][IO.Directory]::CreateDirectory((Join-Path $dir "vrclt/updates"))
        $target = Join-Path $dir "My VRCLT.exe"
        Copy-Item -LiteralPath $starter -Destination $target
        $digest = if ($mode -eq "corrupt") { "0" * 64 } else { $expected }
        $lock = if ($mode -eq "locked") { [IO.File]::Open($target, 'Open', 'Read', 'Read') }
        try {
            & $target $built $digest $mode $dir
            if ($LASTEXITCODE) { throw "Update handshake failed for $mode" }
            $helperId = [int](Get-Content -LiteralPath (Join-Path $dir "helper.pid"))
            $helper = Get-Process -Id $helperId -ErrorAction SilentlyContinue
            if ($helper -and !$helper.WaitForExit(30000)) { throw "Timed out waiting for $mode" }
            $installed = (Get-FileHash -LiteralPath $target).Hash.ToLowerInvariant()
            $want = if ($mode -in @("install", "restart")) { $expected } else { $oldHash }
            if ($installed -ne $want) { throw "Unexpected installed file after $mode" }
            if ($mode -in @("locked", "tampered") -and !(Test-Path -LiteralPath (Join-Path $dir "vrclt/updates/install-error.txt"))) {
                throw "A rejected installation must retain a visible error"
            }
            Write-Host "Windows updater passed: $mode"
        } finally { if ($lock) { $lock.Dispose() } }
    }
} finally {
    $resolved = (Resolve-Path -LiteralPath $testDir).Path
    if ([IO.Path]::GetDirectoryName($resolved) -eq $root.TrimEnd('\') -and [IO.Path]::GetFileName($resolved) -like 'vrclt-update-test-*') {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
