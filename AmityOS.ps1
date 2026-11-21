param(
    # run     boot the AmityOS image in a VM (creates the VM disk on first use)
    # image   build the AmityOS disk image with mkosi (in WSL, as root)
    # build   cargo build of the workspace (in WSL)
    # test    cargo test of the workspace (in WSL)
    # vmtest  boot the development image headless and run the image acceptance checks
    [ValidateSet('run','image','build','test','vmtest')][string]$Command = 'run',
    [switch]$ResetVm,
    [ValidateSet('whpx','tcg')][string]$Accel = 'whpx',
    [ValidateRange(2048, 65536)][int]$MemoryMiB = 8192,
    [ValidateRange(1, 16)][int]$Cpus = 4,
    [switch]$SkipApps,
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
            # The Cargo workspace builds in WSL with the pinned toolchain under .deps\.
            & wsl -d Ubuntu-24.04 --cd /mnt/f/AmityOS -- python3 -B tooling/amity_dev.py $Command
            if ($LASTEXITCODE -ne 0) { throw "AmityOS command failed with exit code $LASTEXITCODE" }
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
            Test-AmityVm -MemoryMiB $MemoryMiB -Cpus $Cpus -Accel $Accel -SkipApps:$SkipApps
        }
    }
} finally {
    if ($AmityLocationPushed) { Pop-Location }
    foreach ($AmityName in $AmityPreviousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($AmityName, $AmityPreviousEnvironment[$AmityName], 'Process')
    }
}
