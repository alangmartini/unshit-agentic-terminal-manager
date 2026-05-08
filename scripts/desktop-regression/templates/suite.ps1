#Requires -Version 5.1

# Copy this file into ../suites/<suite-name>.ps1 and update the metadata.
# The runner dot-sources every suite file and executes the registered
# scriptblock when the suite is selected.

Register-DesktopRegressionSuite `
    -Name "example-suite-name" `
    -Title "Human readable suite title" `
    -Covers "One sentence describing the desktop behavior this suite protects." `
    -Tags @("windows", "visual") `
    -ScriptBlock {
        param($Context)

        $session = Start-DesktopRegressionApp -Context $Context
        try {
            $hwnd = $session.WindowHandle

            # Arrange the desktop state.
            Focus-DesktopRegressionWindow -Handle $hwnd

            # Capture evidence before and after the interaction.
            $shot = New-DesktopRegressionArtifactPath `
                -Context $Context `
                -SuiteName "example-suite-name" `
                -Name "screenshot"
            Capture-DesktopRegressionScreen -Path $shot

            # Assert behavior with explicit failure messages.
            Assert-DesktopRegressionTrue `
                -Condition $true `
                -Message "replace this with the condition this suite protects"
        } finally {
            Stop-DesktopRegressionApp -Session $session
        }
    }
