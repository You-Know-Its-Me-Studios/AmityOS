param(
    # run     boot the AmityOS image in a VM (creates the VM disk on first use)
    # image   build the AmityOS disk image with mkosi (in WSL, as root)
    # build   cargo build of the workspace (in the build root, in WSL as root)
    # test    cargo test of the workspace (same)
    # vmtest  boot the development image headless and run an acceptance suite (-Suite)
    # nested  run the compositor and shell as a window on this desktop (WSLg) and check them
    # licenses  regenerate docs\licensing\crates.md and fail on a licence the rules forbid
    [ValidateSet('run','image','build','test','vmtest','nested','licenses')][string]$Command = 'run',
    [switch]$ResetVm,
    [ValidateSet('whpx','tcg')][string]$Accel = 'whpx',
    [ValidateRange(2048, 65536)][int]$MemoryMiB = 8192,
    [ValidateRange(1, 16)][int]$Cpus = 4,
    [switch]$SkipApps,
    # phase2: image, network, Chrome and VS Code. phase3: compositor and shell, then
    # Chrome, VS Code and Blender from the launcher (-SkipApps leaves those out).
    # phase3-apps: only the applications part. phase3-multi: two monitors.
    # exec: run -Exec commands on the guest.
    [ValidateSet('phase2','phase3','phase3-apps','phase3-multi','exec')][string]$Suite = 'phase2',
    # Reuse the previous test VM disk instead of starting from the image.
    [switch]$KeepVm,
    # Test the binaries exported by "tooling/dev.sh export" instead of those in the image.
    [switch]$DevBuild,
    # With -Suite exec: commands to run on the guest's root console, for diagnosis.
    [string[]]$Exec = @(),
    [switch]$Screenshot,
    # devel: adds a root shell on the serial port for the test harness (VM use only).
    # release: no development console; for writing to real hardware.
    [ValidateSet('devel','release')][string]$ImageProfile = 'devel'
)
$ErrorActionPreference = 'Stop'
# Storage rule: all work, builds, caches, VM disks and downloads stay on F:\AmityOS.
$AmityRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
if ($AmityRoot -cne 'F:\AmityOS' -or ((Get-Item -LiteralPath $PSScriptRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'AmityOS must run from the F:\AmityOS repository. No C: fallback is permitted.'
}
$AmityVolume = [IO.DriveInfo]::new('F:\')
if (-not $AmityVolume.IsReady -or $AmityVolume.DriveFormat -ne 'NTFS') { throw 'AmityOS requires the available NTFS F: volume.' }
$AmityPredicted = switch ($Command) { 'image' { 60GB } 'run' { 8GB } 'vmtest' { 12GB } default { 2GB } }
if ($AmityVolume.AvailableFreeSpace -lt (30GB + $AmityPredicted)) {
    throw "AmityOS stopped: F: needs a 30 GiB free-space reserve plus $($AmityPredicted / 1GB) GiB for this command. No C: fallback."
}
$AmityPreviousEnvironment = @{}
$AmityLocalEnvironment = @{TEMP='F:\AmityOS\.cache\tmp'; TMP='F:\AmityOS\.cache\tmp'; PYTHONDONTWRITEBYTECODE='1'}
foreach ($AmityCacheAncestor in @('F:\AmityOS\.cache','F:\AmityOS\.cache\tmp')) {
    $AmityCacheItem = Get-Item -LiteralPath $AmityCacheAncestor -Force -ErrorAction SilentlyContinue
    if ($AmityCacheItem -and ($AmityCacheItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "AmityOS cache cannot redirect through a link: $AmityCacheAncestor"
    }
}
$AmityLocationPushed = $false
try {
    foreach ($AmityName in $AmityLocalEnvironment.Keys) {
        $AmityPreviousEnvironment[$AmityName] = [Environment]::GetEnvironmentVariable($AmityName, 'Process')
        [Environment]::SetEnvironmentVariable($AmityName, $AmityLocalEnvironment[$AmityName], 'Process')
    }
    [IO.Directory]::CreateDirectory($AmityLocalEnvironment.TEMP) | Out-Null
    Push-Location -LiteralPath $PSScriptRoot
    $AmityLocationPushed = $true
    switch ($Command) {
        { $_ -in 'build','test' } {
            # The Cargo workspace builds with the pinned toolchain under .deps\, inside the
            # build root: an Ubuntu tree of the image's release with the development
            # libraries, which tooling/dev.sh creates on first use. Entering it needs root.
            & wsl -d Ubuntu-24.04 -u root -- bash /mnt/f/AmityOS/tooling/dev.sh cargo $Command --workspace --locked
            if ($LASTEXITCODE -ne 0) { throw "AmityOS command failed with exit code $LASTEXITCODE" }
        }
        'nested' {
            & wsl -d Ubuntu-24.04 -u root -- bash /mnt/f/AmityOS/tooling/dev.sh cargo build --release --locked -p amity-comp -p amity-shell
            if ($LASTEXITCODE -ne 0) { throw "Build failed with exit code $LASTEXITCODE" }
            & wsl -d Ubuntu-24.04 -u root -- bash /mnt/f/AmityOS/tooling/dev.sh run bash tooling/nested_test.sh release
            if ($LASTEXITCODE -ne 0) { throw "The nested session check failed with exit code $LASTEXITCODE" }
        }
        'licenses' {
            & wsl -d Ubuntu-24.04 -u root -- bash /mnt/f/AmityOS/tooling/dev.sh metadata
            if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE" }
            $AmityPython = (Get-Command python -ErrorAction SilentlyContinue).Source
            if (-not $AmityPython) { throw 'The licence ledger needs Python 3 on Windows (python.exe on PATH).' }
            & $AmityPython -B (Join-Path $PSScriptRoot 'tooling\licenses.py') (Join-Path $PSScriptRoot '.cache\tmp\cargo-metadata.json') `
                --write (Join-Path $PSScriptRoot 'docs\licensing\crates.md') --check
            if ($LASTEXITCODE -ne 0) { throw 'A linked dependency has a licence the rules do not allow (see above).' }
        }
        'image' {
            # mkosi assembles a root filesystem, which needs root inside WSL.
            $AmityProfile = if ($ImageProfile -eq 'release') { '' } else { $ImageProfile }
            & wsl -d Ubuntu-24.04 -u root -- env "AMITY_IMAGE_PROFILE=$AmityProfile" bash /mnt/f/AmityOS/tooling/image_build.sh
            if ($LASTEXITCODE -ne 0) { throw "Image build failed with exit code $LASTEXITCODE" }
        }
        'run' {
            . (Join-Path $PSScriptRoot 'tooling\vm.ps1')
            Start-AmityVm -Reset:$ResetVm -MemoryMiB $MemoryMiB -Cpus $Cpus -Accel $Accel
        }
        'vmtest' {
            . (Join-Path $PSScriptRoot 'tooling\vm.ps1')
            Test-AmityVm -MemoryMiB $MemoryMiB -Cpus $Cpus -Accel $Accel -SkipApps:$SkipApps -Suite $Suite -KeepVm:$KeepVm -Exec $Exec -Screenshot:$Screenshot -DevBuild:$DevBuild
        }
    }
} finally {
    if ($AmityLocationPushed) { Pop-Location }
    foreach ($AmityName in $AmityPreviousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($AmityName, $AmityPreviousEnvironment[$AmityName], 'Process')
    }
}
