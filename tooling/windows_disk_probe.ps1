param([Parameter(Mandatory = $true)][string]$ImagePath)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$probeHandle = $null
$result = $null
$probeExit = 1
try {
    # Open existing ordinary VM-image files only. FileShare.None conflicts with
    # any existing read/write/delete handle, even if that handle permits sharing.
    # There is no create, truncate, write, mount, or physical-device operation.
    if ($ImagePath -notmatch '^[A-Za-z]:\\' -or $ImagePath.IndexOf(':', 2) -ge 0) {
        throw 'Expected an ordinary Windows drive path'
    }
    $attributes = [System.IO.File]::GetAttributes($ImagePath)
    if (($attributes -band ([System.IO.FileAttributes]::Directory -bor [System.IO.FileAttributes]::ReparsePoint)) -ne 0) {
        throw 'Expected a regular, non-reparse VM image'
    }
    $probeHandle = [System.IO.File]::Open($ImagePath, [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read, [System.IO.FileShare]::None)
    $result = @{closed = $true; probe = 'read-exclusive'}
    $probeExit = 0
} catch {
    $exception = $_.Exception
    while ($null -ne $exception.InnerException) { $exception = $exception.InnerException }
    $result = @{closed = $false; probe = 'read-exclusive'; win32_error = ($exception.HResult -band 0xffff)}
} finally {
    if ($null -ne $probeHandle) { $probeHandle.Dispose() }
}
$result | ConvertTo-Json -Compress
exit $probeExit
