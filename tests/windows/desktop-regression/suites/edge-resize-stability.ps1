#Requires -Version 5.1

Register-DesktopRegressionSuite `
    -Name "edge-resize-stability" `
    -Title "Frameless left-edge resize stability" `
    -Covers "Dragging the frameless left resize edge inward and back must keep the right edge stable and restore the original left edge." `
    -Tags @("windows", "resize", "frameless-window", "input") `
    -ScriptBlock {
        param($Context)

        $session = Start-DesktopRegressionApp -Context $Context
        try {
            $hwnd = $session.WindowHandle
            $screen = Get-DesktopRegressionScreenSize

            $targetW = [int][Math]::Round($screen.Width / 2.0)
            $targetH = [int][Math]::Max(500, $screen.Height * 0.88)
            Set-DesktopRegressionWindowRect `
                -Handle $hwnd `
                -X 0 `
                -Y 0 `
                -Width $targetW `
                -Height $targetH
            Start-Sleep -Milliseconds 700

            Focus-DesktopRegressionWindow -Handle $hwnd

            $shotStart = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "edge-resize-stability" `
                -Name "start"
            $shotAfter = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "edge-resize-stability" `
                -Name "after"
            $shotRestore = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "edge-resize-stability" `
                -Name "restore"

            $r0 = Get-DesktopRegressionRect -Handle $hwnd
            Capture-DesktopRegressionScreen -Path $shotStart
            Write-Output ("initial_rect={0}" -f (Format-DesktopRegressionRect -Rect $r0))

            $centerY = [int][Math]::Round(($r0.Top + $r0.Bottom) / 2)
            $leftX = $r0.Left + 4
            $dragToX = [int][Math]::Min($r0.Right - 20, $leftX + $Context.DragDelta)

            Invoke-DesktopRegressionLeftEdgeDrag `
                -Handle $hwnd `
                -FromY $centerY `
                -FromX $leftX `
                -ToX $dragToX
            $r1 = Get-DesktopRegressionRect -Handle $hwnd
            Capture-DesktopRegressionScreen -Path $shotAfter

            $restoreX = [int][Math]::Max(0, $r0.Left + 4)
            $restoreFromX = $r1.Left + 4
            Invoke-DesktopRegressionLeftEdgeDrag `
                -Handle $hwnd `
                -FromY $centerY `
                -FromX $restoreFromX `
                -ToX $restoreX
            $r2 = Get-DesktopRegressionRect -Handle $hwnd
            Capture-DesktopRegressionScreen -Path $shotRestore

            Write-Output ("after_rect={0}" -f (Format-DesktopRegressionRect -Rect $r1))
            Write-Output ("restore_rect={0}" -f (Format-DesktopRegressionRect -Rect $r2))
            Write-Output ("screenshots:{0};{1};{2}" -f $shotStart, $shotAfter, $shotRestore)

            Assert-DesktopRegressionClose `
                -Actual $r1.Right `
                -Expected $r0.Right `
                -Tolerance $Context.Tolerance `
                -Name "after-right-edge"
            Assert-DesktopRegressionClose `
                -Actual $r2.Right `
                -Expected $r0.Right `
                -Tolerance $Context.Tolerance `
                -Name "restore-right-edge"
            Assert-DesktopRegressionTrue `
                -Condition ($r1.Left -gt $r0.Left) `
                -Message "left edge did not move right during inward resize"
            Assert-DesktopRegressionTrue `
                -Condition ([Math]::Abs($r2.Left - $r0.Left) -le $Context.Tolerance) `
                -Message "left edge did not return near origin after outward resize"
        } finally {
            Stop-DesktopRegressionApp -Session $session
        }
    }
