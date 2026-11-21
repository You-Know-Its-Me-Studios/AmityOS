# VM launch for the AmityOS image with the pinned Windows QEMU. Dot-sourced by
# AmityOS.ps1. All state stays under F:\AmityOS\image\out\vm.
$ErrorActionPreference = 'Stop'
$script:AmityRoot = 'F:\AmityOS'
$script:QemuDir = Join-Path $AmityRoot '.deps\qemu-windows-8.2.0\runtime'
$script:Qemu = Join-Path $QemuDir 'qemu-system-x86_64.exe'
$script:QemuImg = Join-Path $QemuDir 'qemu-img.exe'
$script:ImageOut = Join-Path $AmityRoot 'image\out'
$script:VmDir = Join-Path $ImageOut 'vm'

function Get-AmityBaseImage {
    param([string]$ImageProfile = 'devel')
    $base = Get-ChildItem -LiteralPath $ImageOut -Filter "amityos-*-$ImageProfile.qcow2" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $base) { throw "No AmityOS image found in $ImageOut. Build one first: .\AmityOS.ps1 image" }
    $base.FullName
}

function Initialize-AmityVm {
    # Creates the persistent VM disk (a qcow2 overlay on the built image, so the
    # build output stays pristine). An overlay made for an older build is
    # refused, never silently replaced: it holds user data.
    param([string]$Name = 'amityos', [int]$DiskGiB = 64, [switch]$Reset)
    foreach ($tool in $Qemu, $QemuImg) { if (-not (Test-Path -LiteralPath $tool)) { throw "Pinned QEMU is missing: $tool" } }
    $base = Get-AmityBaseImage
    $baseHash = (Get-Content -LiteralPath "$base.sha256" -TotalCount 1).Split(' ')[0]
    [IO.Directory]::CreateDirectory($VmDir) | Out-Null
    $disk = Join-Path $VmDir "$Name.qcow2"
    $stamp = Join-Path $VmDir "$Name.base"
    if ($Reset) { Remove-Item -LiteralPath $disk, $stamp -Force -ErrorAction SilentlyContinue }
    # UEFI firmware as one read-only ROM image (variable store + code from the
    # pinned QEMU bundle). WHPX cannot emulate the flash-device accesses OVMF
    # makes with the usual pflash setup ("Failed to emulate MMIO access"), so
    # UEFI variables are not persistent; the image boots from the fallback path.
    $firmware = Join-Path $VmDir 'ovmf-combined.fd'
    if (-not (Test-Path -LiteralPath $firmware)) {
        $bytes = [IO.File]::ReadAllBytes((Join-Path $QemuDir 'share\edk2-i386-vars.fd')) + [IO.File]::ReadAllBytes((Join-Path $QemuDir 'share\edk2-x86_64-code.fd'))
        if ($bytes.Length -ne 4MB) { throw 'Unexpected pinned OVMF firmware size' }
        [IO.File]::WriteAllBytes($firmware, $bytes)
    }
    if (Test-Path -LiteralPath $disk) {
        $made = if (Test-Path -LiteralPath $stamp) { (Get-Content -LiteralPath $stamp -TotalCount 1) } else { '' }
        if ($made -ne $baseHash) {
            throw "The VM disk $disk was created from a different image build. It may hold your data, so it is kept. To start a fresh VM from the new image run: .\AmityOS.ps1 run -ResetVm"
        }
    } else {
        & $QemuImg create -q -f qcow2 -F qcow2 -b $base $disk "${DiskGiB}G"
        if ($LASTEXITCODE -ne 0) { throw 'qemu-img could not create the VM disk' }
        Set-Content -LiteralPath $stamp -Value $baseHash -Encoding ascii
    }
    [pscustomobject]@{ Disk = $disk; Firmware = $firmware; Base = $base }
}

function Get-AmityVmArguments {
    param([Parameter(Mandatory)]$Vm, [int]$MemoryMiB = 8192, [int]$Cpus = 4, [string]$Display = 'gtk',
          [ValidateSet('whpx','tcg')][string]$Accel = 'whpx', [int]$SerialPort = 0, [int]$QmpPort = 0,
          [int]$Outputs = 1, [int]$VncPort = 0)
    $accelArg = if ($Accel -eq 'whpx') { 'whpx,kernel-irqchip=off' } else { 'tcg,thread=multi' }
    # "-cpu max" crashes OVMF under WHPX with this QEMU; this model boots.
    $cpu = if ($Accel -eq 'whpx') { 'qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt' } else { 'max' }
    $arguments = @(
        '-name', 'AmityOS', '-machine', 'q35', '-accel', $accelArg, '-cpu', $cpu,
        '-smp', "$Cpus", '-m', "$MemoryMiB", '-L', (Join-Path $QemuDir 'share'),
        '-bios', $Vm.Firmware,
        '-drive', "if=none,id=system,format=qcow2,cache=writeback,discard=unmap,file=$($Vm.Disk)",
        '-device', 'virtio-blk-pci,drive=system,bootindex=1',
        '-device', "virtio-vga,id=video0,max_outputs=$Outputs", '-display', $Display,
        '-device', 'virtio-keyboard-pci', '-device', 'virtio-tablet-pci',
        '-netdev', 'user,id=net0', '-device', 'virtio-net-pci,netdev=net0',
        '-device', 'virtio-rng-pci', '-monitor', 'none'
    )
    # The harness reaches the guest only through loopback TCP on this machine.
    if ($SerialPort) { $arguments += @('-serial', "tcp:127.0.0.1:$SerialPort,server=on,wait=off") } else { $arguments += @('-serial', 'none') }
    if ($QmpPort) { $arguments += @('-qmp', "tcp:127.0.0.1:$QmpPort,server=on,wait=off") }
    # A second display head stays disconnected until a display front end announces a
    # size for it. Headless, that front end is a VNC server bound to the head; the test
    # harness connects to it (loopback only) to plug the second monitor in and out.
    if ($VncPort) { $arguments += @('-vnc', "127.0.0.1:$($VncPort - 5900),display=video0,head=1") }
    $arguments
}

function Start-AmityVm {
    param([switch]$Reset, [int]$MemoryMiB = 8192, [int]$Cpus = 4, [ValidateSet('whpx','tcg')][string]$Accel = 'whpx')
    $vm = Initialize-AmityVm -Reset:$Reset
    $arguments = Get-AmityVmArguments -Vm $vm -MemoryMiB $MemoryMiB -Cpus $Cpus -Accel $Accel
    Write-Host "Starting AmityOS VM ($Accel, $Cpus CPUs, $MemoryMiB MiB). Disk: $($vm.Disk)"
    & $Qemu @arguments
    if ($LASTEXITCODE -ne 0) { throw "QEMU exited with code $LASTEXITCODE" }
}

function Get-AmityFreePort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop(); $port
}

function Test-AmityVm {
    # Boots a fresh, disposable test VM from the development image without a
    # window and runs tooling\vm_harness.py against its serial console and QMP.
    param([int]$MemoryMiB = 8192, [int]$Cpus = 4, [ValidateSet('whpx','tcg')][string]$Accel = 'whpx', [switch]$SkipApps,
          [ValidateSet('phase2','phase3','phase3-apps','phase3-multi','exec')][string]$Suite = 'phase2', [switch]$KeepVm,
          [string[]]$Exec = @(), [switch]$Screenshot, [switch]$DevBuild)
    $python = (Get-Command python -ErrorAction SilentlyContinue).Source
    if (-not $python) { throw 'The image acceptance harness needs Python 3 on Windows (python.exe on PATH).' }
    # -KeepVm reuses the previous test VM disk (apps already installed) for faster iteration.
    $vm = Initialize-AmityVm -Name 'amityos-test' -Reset:(-not $KeepVm)
    $serial = Get-AmityFreePort; $qmp = Get-AmityFreePort
    $stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ')
    $out = Join-Path $ImageOut "acceptance\$stamp"
    [IO.Directory]::CreateDirectory($out) | Out-Null
    $outputs = 1; $vnc = 0
    if ($Suite -eq 'phase3-multi') {
        $outputs = 2
        do { $vnc = Get-AmityFreePort } while ($vnc -lt 5900)
    }
    $arguments = Get-AmityVmArguments -Vm $vm -MemoryMiB $MemoryMiB -Cpus $Cpus -Accel $Accel -Display 'none' `
        -SerialPort $serial -QmpPort $qmp -Outputs $outputs -VncPort $vnc
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden `
        -RedirectStandardError (Join-Path $out 'qemu.stderr.log') -RedirectStandardOutput (Join-Path $out 'qemu.stdout.log')
    try {
        $harness = @('-B', (Join-Path $AmityRoot 'tooling\vm_harness.py'), '--serial-port', $serial, '--qmp-port', $qmp, '--out', $out)
        if ($SkipApps) { $harness += '--skip-apps' }
        $harness += @('--suite', $Suite)
        foreach ($line in $Exec) { $harness += @('--exec', $line) }
        if ($Screenshot) { $harness += '--screenshot' }
        if ($vnc) { $harness += @('--vnc-port', $vnc) }
        # -DevBuild tests the binaries last exported by "tooling/dev.sh export" instead of the
        # ones in the image, to iterate without rebuilding the image. Diagnostics (exec) may
        # fetch files from there as well.
        $bin = Join-Path $ImageOut 'bin'
        if ($DevBuild -or $Suite -eq 'exec') {
            if (-not (Test-Path -LiteralPath $bin)) { throw "No exported build in $bin (run tooling/dev.sh export)." }
            $harness += @('--serve', $bin)
        }
        & $python @harness
        $code = $LASTEXITCODE
        # The harness asked the guest to power off, whether it passed or failed; give it
        # time to do so cleanly, so that the test VM's disk keeps what the run wrote. A
        # failed run may have left a guest that cannot power off, so it gets less time.
        $grace = if ($code -eq 0) { 120000 } else { 20000 }
        if (-not $process.WaitForExit($grace)) { Write-Warning "The guest did not power off within $($grace / 1000) s; stopping QEMU." }
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    Write-Host "Acceptance record: $out\results.json"
    if ($code -ne 0) { throw "Image acceptance failed (exit code $code). See $out" }
}
