[CmdletBinding()]
param(
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$releaseDir = Join-Path $repoRoot "src-tauri\target\release"
$releaseExe = Join-Path $releaseDir "investa.exe"
$launcherDataDir = Join-Path $env:LOCALAPPDATA "Investa\launcher"
$launcherLog = Join-Path $launcherDataDir "launcher.log"
$mutex = [Threading.Mutex]::new($false, "Local\InvestaDesktopLauncherBuild")
$hasMutex = $false

function Write-LauncherLog {
    param([Parameter(Mandatory = $true)][string]$Message)

    New-Item -ItemType Directory -Force -Path $launcherDataDir | Out-Null
    $line = "{0} {1}" -f (Get-Date).ToString("o"), $Message
    Add-Content -LiteralPath $launcherLog -Value $line -Encoding utf8
}

function Get-LatestBuildInputUtc {
    $inputDirectories = @(
        (Join-Path $repoRoot "src"),
        (Join-Path $repoRoot "src-tauri\src"),
        (Join-Path $repoRoot "src-tauri\capabilities"),
        (Join-Path $repoRoot "ml-worker")
    )
    $inputFiles = @(
        (Join-Path $repoRoot "index.html"),
        (Join-Path $repoRoot "package.json"),
        (Join-Path $repoRoot "pnpm-lock.yaml"),
        (Join-Path $repoRoot "vite.config.ts"),
        (Join-Path $repoRoot "tsconfig.json"),
        (Join-Path $repoRoot "tsconfig.app.json"),
        (Join-Path $repoRoot "tsconfig.node.json"),
        (Join-Path $repoRoot "src-tauri\build.rs"),
        (Join-Path $repoRoot "src-tauri\Cargo.toml"),
        (Join-Path $repoRoot "src-tauri\Cargo.lock"),
        (Join-Path $repoRoot "src-tauri\tauri.conf.json")
    )

    $candidates = [Collections.Generic.List[IO.FileInfo]]::new()
    foreach ($directory in $inputDirectories) {
        if (Test-Path -LiteralPath $directory) {
            Get-ChildItem -LiteralPath $directory -Recurse -File | ForEach-Object {
                $candidates.Add($_)
            }
        }
    }
    foreach ($file in $inputFiles) {
        if (Test-Path -LiteralPath $file) {
            $candidates.Add((Get-Item -LiteralPath $file))
        }
    }

    if ($candidates.Count -eq 0) {
        throw "No build inputs were found under: $repoRoot"
    }

    return ($candidates | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).LastWriteTimeUtc
}

function Find-PnpmCommand {
    $nodeCommand = Get-Command node.exe -ErrorAction SilentlyContinue
    if ($null -eq $nodeCommand) {
        $nodeCandidates = @(
            (Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe"),
            (Join-Path $env:ProgramFiles "nodejs\node.exe"),
            (Join-Path $env:LOCALAPPDATA "Programs\nodejs\node.exe")
        )
        if (-not [string]::IsNullOrWhiteSpace(${env:ProgramFiles(x86)})) {
            $nodeCandidates += Join-Path ${env:ProgramFiles(x86)} "nodejs\node.exe"
        }

        $nodeExe = $nodeCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if ($null -eq $nodeExe) {
            throw "node.exe was not found. Install Node.js or restore the bundled workspace runtime, then retry."
        }
        $nodeDirectory = Split-Path -Parent $nodeExe
        $env:Path = "$nodeDirectory;$env:Path"
    }

    $command = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $runtimeRoot = Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies"
    $fallbackPnpm = Join-Path $runtimeRoot "bin\fallback\pnpm.cmd"
    $fallbackNode = Join-Path $runtimeRoot "node\bin"
    if ((Test-Path -LiteralPath $fallbackPnpm) -and (Test-Path -LiteralPath (Join-Path $fallbackNode "node.exe"))) {
        $env:Path = "$fallbackNode;$env:Path"
        return $fallbackPnpm
    }

    throw "pnpm was not found. Install Node.js and pnpm, then retry."
}

function Invoke-ReleaseBuild {
    param([Parameter(Mandatory = $true)][string]$PnpmPath)

    # Tauri runs beforeBuildCommand as a child process. The shortcut may find the
    # bundled pnpm by absolute path even though its directory is absent from PATH.
    # Add only that already-resolved executable directory so the nested `pnpm build`
    # uses the same trusted runtime instead of failing or resolving another shim.
    $pnpmDirectory = Split-Path -Parent $PnpmPath
    $pathEntries = $env:Path -split [IO.Path]::PathSeparator
    if ($pathEntries -notcontains $pnpmDirectory) {
        $env:Path = "$pnpmDirectory$([IO.Path]::PathSeparator)$env:Path"
    }

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $env:ComSpec
    $startInfo.Arguments = "/d /s /c `"`"$PnpmPath`" tauri build --no-bundle`""
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "The Investa release build process did not start."
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit([int][TimeSpan]::FromMinutes(15).TotalMilliseconds)) {
            try { $process.Kill() } catch {}
            throw "The Investa release build exceeded 15 minutes and was stopped."
        }
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        foreach ($output in @($stdout, $stderr)) {
            if (-not [string]::IsNullOrWhiteSpace($output)) {
                Add-Content -LiteralPath $launcherLog -Value $output -Encoding utf8
            }
        }

        if ($process.ExitCode -ne 0) {
            throw "The Investa release build failed with exit code $($process.ExitCode)."
        }
    }
    finally {
        $process.Dispose()
    }
}

function Show-LauncherError {
    param([Parameter(Mandatory = $true)][string]$Message)

    try {
        Add-Type -AssemblyName PresentationFramework
        [System.Windows.MessageBox]::Show(
            "$Message`n`nLog: $launcherLog",
            "Investa launch failed",
            [System.Windows.MessageBoxButton]::OK,
            [System.Windows.MessageBoxImage]::Error
        ) | Out-Null
    }
    catch {
        Write-Error $Message
    }
}

try {
    $hasMutex = $mutex.WaitOne([TimeSpan]::FromMinutes(10))
    if (-not $hasMutex) {
        throw "Timed out while waiting for another Investa build."
    }

    $latestInputUtc = Get-LatestBuildInputUtc
    $exeExists = Test-Path -LiteralPath $releaseExe
    $exeWriteUtc = if ($exeExists) { (Get-Item -LiteralPath $releaseExe).LastWriteTimeUtc } else { $null }
    $needsBuild = (-not $exeExists) -or ($exeWriteUtc -lt $latestInputUtc)

    if ($CheckOnly) {
        $runtimeReady = $true
        $runtimeError = $null
        $pnpmPath = $null
        $nodePath = $null
        try {
            $pnpmPath = Find-PnpmCommand
            $nodePath = (Get-Command node.exe -ErrorAction Stop).Source
        }
        catch {
            $runtimeReady = $false
            $runtimeError = $_.Exception.Message
        }
        [pscustomobject]@{
            repoRoot = $repoRoot
            releaseExe = $releaseExe
            exeExists = $exeExists
            needsBuild = $needsBuild
            runtimeReady = $runtimeReady
            nodePath = $nodePath
            pnpmPath = $pnpmPath
            runtimeError = $runtimeError
            latestInputUtc = $latestInputUtc.ToString("o")
            exeWriteUtc = if ($null -ne $exeWriteUtc) { $exeWriteUtc.ToString("o") } else { $null }
        } | ConvertTo-Json -Compress
        exit 0
    }

    if ($needsBuild) {
        $pnpm = Find-PnpmCommand
        Write-LauncherLog "The release is older than its inputs; starting an incremental build."
        Invoke-ReleaseBuild -PnpmPath $pnpm
        Write-LauncherLog "The incremental release build completed."
    }

    $runningWindow = Get-Process -Name "investa" -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $releaseExe -and $_.MainWindowHandle -ne 0 } |
        Select-Object -First 1
    if ($null -ne $runningWindow) {
        (New-Object -ComObject WScript.Shell).AppActivate($runningWindow.Id) | Out-Null
        exit 0
    }

    if (-not (Test-Path -LiteralPath $releaseExe)) {
        throw "The release executable was not found after the build: $releaseExe"
    }

    Start-Process -FilePath $releaseExe -WorkingDirectory $releaseDir | Out-Null
    Write-LauncherLog "Investa started."
}
catch {
    $message = $_.Exception.Message
    Write-LauncherLog "Failure: $message"
    Show-LauncherError -Message $message
    exit 1
}
finally {
    if ($hasMutex) {
        $mutex.ReleaseMutex()
    }
    $mutex.Dispose()
}
