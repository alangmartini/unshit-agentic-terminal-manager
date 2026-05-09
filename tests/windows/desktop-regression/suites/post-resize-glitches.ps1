#Requires -Version 5.1

Register-DesktopRegressionSuite `
    -Name "post-resize-glitches" `
    -Title "Post-resize terminal viewport glitches" `
    -Covers "Aero snap growth must move chrome to the new bottom and must not leave stale terminal rows floating in the enlarged viewport." `
    -Tags @("windows", "snap", "resize", "visual", "terminal-grid") `
    -ScriptBlock {
        param($Context)

        $session = Start-DesktopRegressionApp -Context $Context
        try {
            $hwnd = $session.WindowHandle
            $screen = Get-DesktopRegressionScreenSize

            $preW = [int][Math]::Round($screen.Width / 2.5)
            $preH = [int][Math]::Round($screen.Height / 2.0)
            Set-DesktopRegressionWindowRect `
                -Handle $hwnd `
                -X 200 `
                -Y 200 `
                -Width $preW `
                -Height $preH
            Start-Sleep -Milliseconds 800

            Focus-DesktopRegressionWindow -Handle $hwnd
            Send-DesktopRegressionClearCommand -Shell $Context.SnapShell
            Start-Sleep -Milliseconds 500
            Focus-DesktopRegressionWindow -Handle $hwnd

            $rPre = Get-DesktopRegressionRect -Handle $hwnd
            $shotPre = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "post-resize-glitches" `
                -Name "pre"
            $shotPost = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "post-resize-glitches" `
                -Name "post"
            Capture-DesktopRegressionScreen -Path $shotPre

            Focus-DesktopRegressionWindow -Handle $hwnd
            Send-DesktopRegressionWinLeft
            Start-Sleep -Milliseconds 1500

            $rPost = Get-DesktopRegressionRect -Handle $hwnd
            Assert-DesktopRegressionTrue `
                -Condition (($rPost.Bottom - $rPost.Top) -gt ($rPre.Bottom - $rPre.Top)) `
                -Message ("Win+Left did not grow window height: pre={0} post={1}" -f `
                    ($rPre.Bottom - $rPre.Top), ($rPost.Bottom - $rPost.Top))

            Capture-DesktopRegressionScreen -Path $shotPost
            Write-Output ("snap_pre_rect={0}" -f (Format-DesktopRegressionRect -Rect $rPre))
            Write-Output ("snap_post_rect={0}" -f (Format-DesktopRegressionRect -Rect $rPost))

            $stripeHeight = $Context.SnapStripeHeightPx
            $bottomStripeY = $rPost.Bottom - $stripeHeight - 4
            $bottomStripeY = [int][Math]::Max($rPost.Top, $bottomStripeY)
            $stripeX = $rPost.Left
            $stripeW = $rPost.Right - $rPost.Left

            $paneLeft = $rPost.Left + $Context.SnapSidebarPx + 140
            $paneTop = $rPost.Top + $Context.SnapTabbarPx
            $paneBottom = $rPost.Bottom - $Context.SnapStatusbarPx
            $paneHeight = $paneBottom - $paneTop
            $midX = [int][Math]::Min($rPost.Right - 1, $paneLeft)
            $midY = [int]($paneTop + ($paneHeight * 0.22))
            $midW = [int][Math]::Min(480, $rPost.Right - $midX)
            $midH = [int][Math]::Max($Context.SnapStripeHeightPx, $paneHeight * 0.56)

            $bmp = [System.Drawing.Bitmap]::FromFile($shotPost)
            try {
                $bottomLit = Get-DesktopRegressionStripeLitRatio `
                    -Bitmap $bmp `
                    -X $stripeX `
                    -Y $bottomStripeY `
                    -Width $stripeW `
                    -Height $stripeHeight
                $midMaxLit = Get-DesktopRegressionMaxStripeLitRatio `
                    -Bitmap $bmp `
                    -X $midX `
                    -Y $midY `
                    -Width $midW `
                    -Height $midH `
                    -StripeHeight $Context.SnapStripeHeightPx `
                    -StepPx $Context.SnapStripeHeightPx
            } finally {
                $bmp.Dispose()
            }

            Write-Output ("snap_bottom_lit_ratio={0:N4} threshold={1:N4} sample=({2},{3} {4}x{5})" -f `
                $bottomLit, $Context.SnapLitRatioThreshold, $stripeX, $bottomStripeY, $stripeW, $stripeHeight)
            Write-Output ("snap_mid_max_lit_ratio={0:N4} threshold={1:N4} sample=({2},{3} {4}x{5})" -f `
                $midMaxLit, $Context.SnapMidLitRatioThreshold, $midX, $midY, $midW, $midH)
            Write-Output ("screenshots:{0};{1}" -f $shotPre, $shotPost)

            Assert-DesktopRegressionTrue `
                -Condition ($bottomLit -ge $Context.SnapLitRatioThreshold) `
                -Message ("snap-resize regression: bottom stripe lit ratio {0:N4} < {1:N4}; statusbar did not reflow to the new window bottom" -f `
                    $bottomLit, $Context.SnapLitRatioThreshold)
            Assert-DesktopRegressionTrue `
                -Condition ($midMaxLit -le $Context.SnapMidLitRatioThreshold) `
                -Message ("snap-resize regression: mid-pane lit ratio {0:N4} > {1:N4}; stale terminal rows appeared in the enlarged viewport" -f `
                    $midMaxLit, $Context.SnapMidLitRatioThreshold)
        } finally {
            Stop-DesktopRegressionApp -Session $session
        }
    }
