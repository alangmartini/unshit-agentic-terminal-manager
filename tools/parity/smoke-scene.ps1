param(
    [int]$HoldSeconds = 20,
    [int]$InitialDelayMs = 1500,
    [int]$StableSizeMs = 750,
    [int]$MaxSizeWaitMs = 3000,
    [string]$MetaPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$esc = [char]27
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
if ($PSVersionTable.PSVersion.Major -ge 7) {
    $PSStyle.OutputRendering = 'Ansi'
}

function Write-Raw {
    param([Parameter(Mandatory)][string]$Text)
    [Console]::Write($Text)
}

function Get-ConsoleSizeSnapshot {
    try {
        return [pscustomobject]@{
            cols = [Console]::WindowWidth
            rows = [Console]::WindowHeight
            buffer_cols = [Console]::BufferWidth
            buffer_rows = [Console]::BufferHeight
        }
    } catch {
        return [pscustomobject]@{
            cols = 0
            rows = 0
            buffer_cols = 0
            buffer_rows = 0
        }
    }
}

function Wait-ConsoleSizeStable {
    param(
        [int]$StableMilliseconds,
        [int]$MaxMilliseconds
    )

    $started = [DateTime]::UtcNow
    $lastChanged = $started
    $last = Get-ConsoleSizeSnapshot
    $current = $last

    if ($StableMilliseconds -le 0 -or $MaxMilliseconds -le 0) {
        return [pscustomobject]@{
            initial = $last
            final = $current
            waited_ms = 0
            stable = $false
        }
    }

    while ((([DateTime]::UtcNow - $started).TotalMilliseconds) -lt $MaxMilliseconds) {
        Start-Sleep -Milliseconds 100
        $current = Get-ConsoleSizeSnapshot
        if (
            $current.cols -ne $last.cols -or
            $current.rows -ne $last.rows -or
            $current.buffer_cols -ne $last.buffer_cols -or
            $current.buffer_rows -ne $last.buffer_rows
        ) {
            $last = $current
            $lastChanged = [DateTime]::UtcNow
        }
        if ((([DateTime]::UtcNow - $lastChanged).TotalMilliseconds) -ge $StableMilliseconds) {
            return [pscustomobject]@{
                initial = $last
                final = $current
                waited_ms = [int](([DateTime]::UtcNow - $started).TotalMilliseconds)
                stable = $true
            }
        }
    }

    [pscustomobject]@{
        initial = $last
        final = $current
        waited_ms = [int](([DateTime]::UtcNow - $started).TotalMilliseconds)
        stable = $false
    }
}

function Write-SceneMeta {
    param(
        [string]$Path,
        $SizeWait
    )

    if ($Path -eq '') {
        return
    }

    $dir = Split-Path -Parent $Path
    if ($dir -ne '') {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }

    [pscustomobject]@{
        generated_at = [DateTime]::UtcNow.ToString('o')
        pid = $PID
        initial_delay_ms = $InitialDelayMs
        stable_size_ms = $StableSizeMs
        max_size_wait_ms = $MaxSizeWaitMs
        size_wait = $SizeWait
        size_before_output = Get-ConsoleSizeSnapshot
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Path -Encoding UTF8
}

try {
    if ($InitialDelayMs -gt 0) {
        Start-Sleep -Milliseconds $InitialDelayMs
    }
    $sizeWait = Wait-ConsoleSizeStable -StableMilliseconds $StableSizeMs -MaxMilliseconds $MaxSizeWaitMs
    Write-SceneMeta -Path $MetaPath -SizeWait $sizeWait

    Write-Raw "$esc[?25l$esc[2J$esc[H$esc[0m"
    Write-Raw "PARITY SMOKE 2026-04-29 | ascii 0123456789 !?[]{}<>`r`n"
    Write-Raw "$esc[1mbold$esc[0m $esc[3mitalic$esc[0m $esc[4munderline$esc[0m $esc[7minverse$esc[0m $esc[9mstrike$esc[0m | "
    Write-Raw "16 fg "
    for ($i = 30; $i -le 37; $i++) {
        Write-Raw "$esc[${i}m $i $esc[0m"
    }
    Write-Raw "`r`n16 bg "
    for ($i = 40; $i -le 47; $i++) {
        Write-Raw "$esc[${i}m  $esc[0m"
    }
    Write-Raw " | bright "
    for ($i = 90; $i -le 97; $i++) {
        Write-Raw "$esc[${i}m $i $esc[0m"
    }
    Write-Raw "`r`n256 ramp "
    foreach ($i in @(16, 17, 18, 19, 20, 21, 27, 33, 39, 45, 51, 87, 123, 159, 195, 231)) {
        Write-Raw "$esc[48;5;${i}m  $esc[0m"
    }
    Write-Raw " | truecolor "
    Write-Raw "$esc[38;2;255;92;92mred$esc[0m "
    Write-Raw "$esc[38;2;75;195;255mblue$esc[0m "
    Write-Raw "$esc[38;2;128;230;128mgreen$esc[0m "
    Write-Raw "$esc[48;2;42;42;54m bg-swatch $esc[0m`r`n"
    Write-Raw "┌──────────────┬──────────────┬──────────────┐`r`n"
    Write-Raw "│ light lines  │ double ╔══╗  │ mixed ╟──╢   │`r`n"
    Write-Raw "├──────────────┼──────────────┼──────────────┤`r`n"
    Write-Raw "│ blocks █▓▒░  │ shade ░▒▓█   │ wide ━━━━━   │`r`n"
    Write-Raw "└──────────────┴──────────────┴──────────────┘`r`n"
    Write-Raw "wrap sentinel 1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ`r`n"
    Write-Raw "$esc[0m"

    if ($HoldSeconds -gt 0) {
        Start-Sleep -Seconds $HoldSeconds
    }
} finally {
    Write-Raw "$esc[0m$esc[?25h"
}
