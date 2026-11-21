# Read-only native accounting for the fixed AmityOS checkout. DirectoryInfo
# enumeration supplies cached file metadata; no per-file WSL/DrvFS stat calls.
# Full logical lengths count sparse files and each hardlink conservatively.
# Reparse entries are skipped, never followed into another tree or volume.
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

try {
    $rootPath = 'F:\AmityOS'
    $root = [System.IO.DirectoryInfo]::new($rootPath)
    $root.Refresh()
    if (-not $root.Exists -or
        ($root.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
        -not [string]::Equals($root.FullName, $rootPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'The actual F:\AmityOS directory is unavailable or redirects elsewhere.'
    }

    $prefix = $rootPath + '\'
    $pending = [System.Collections.Generic.Stack[System.IO.DirectoryInfo]]::new()
    $pending.Push($root)
    [long]$total = 0
    [long]$fileCount = 0
    [long]$skipped = 0
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        # Recheck queued directories so a newly created junction is not walked.
        $directory.Refresh()
        if (-not $directory.Exists) {
            throw ('Directory disappeared during storage accounting: ' + $directory.FullName)
        }
        if (($directory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            $skipped++
            continue
        }
        if (-not [string]::Equals($directory.FullName, $rootPath, [System.StringComparison]::OrdinalIgnoreCase) -and
            -not $directory.FullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw 'Storage enumeration escaped the fixed project root.'
        }
        foreach ($entry in $directory.EnumerateFileSystemInfos()) {
            if (-not $entry.FullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw 'Storage enumeration returned an entry outside the fixed project root.'
            }
            if (($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                $skipped++
                continue
            }
            if (($entry.Attributes -band [System.IO.FileAttributes]::Directory) -ne 0) {
                $pending.Push([System.IO.DirectoryInfo]$entry)
            } elseif ($entry -is [System.IO.FileInfo]) {
                [long]$length = $entry.Length
                if ($length -lt 0 -or $length -gt ([long]::MaxValue - $total)) {
                    throw 'Project logical storage size exceeds the supported integer range.'
                }
                $total += $length
                $fileCount++
            } else {
                throw ('Unsupported filesystem entry: ' + $entry.FullName)
            }
        }
    }
    [ordered]@{
        version = 1
        root = $rootPath
        project_bytes = $total
        file_count = $fileCount
        skipped_reparse = $skipped
    } | ConvertTo-Json -Compress
    exit 0
} catch {
    [Console]::Error.WriteLine('AmityOS storage accounting failed: ' + $_.Exception.Message)
    exit 1
}
