param(
    [ValidateSet('run','build','test')][string]$Command = 'run'
)
$ErrorActionPreference = 'Stop'
# Storage rule: all work, builds, caches, VM disks and downloads stay on F:\AmityOS.
$AmityRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
if ($AmityRoot -cne 'F:\AmityOS' -or ((Get-Item -LiteralPath $PSScriptRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'AmityOS must run from the F:\AmityOS repository. No C: fallback is permitted.'
}
$AmityVolume = [IO.DriveInfo]::new('F:\')
if (-not $AmityVolume.IsReady -or $AmityVolume.DriveFormat -ne 'NTFS') { throw 'AmityOS requires the available NTFS F: volume.' }
$AmityPredicted = if ($Command -eq 'run') { 1GB } else { 2GB }
if ($AmityVolume.AvailableFreeSpace -lt (30GB + $AmityPredicted)) {
    throw "AmityOS stopped: F: needs a 30 GiB free-space reserve plus $($AmityPredicted / 1GB) GiB for this command. No C: fallback."
}
if ($Command -eq 'run') {
    throw 'There is no bootable AmityOS image yet. The Linux-based image and its VM launch arrive in Phase 2 (docs/roadmap.md). Use: .\AmityOS.ps1 build  or  .\AmityOS.ps1 test'
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
    # The Cargo workspace builds in WSL with the pinned toolchain under .deps\.
    & wsl -d Ubuntu-24.04 --cd /mnt/f/AmityOS -- python3 -B tooling/amity_dev.py $Command
    if ($LASTEXITCODE -ne 0) { throw "AmityOS command failed with exit code $LASTEXITCODE" }
} finally {
    if ($AmityLocationPushed) { Pop-Location }
    foreach ($AmityName in $AmityPreviousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($AmityName, $AmityPreviousEnvironment[$AmityName], 'Process')
    }
}
