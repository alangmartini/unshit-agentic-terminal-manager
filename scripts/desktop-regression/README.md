# Desktop Interaction Regression

This path is kept as a compatibility entry point.

The canonical framework lives in:

```powershell
tests\windows\desktop-regression
```

Prefer:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List
```

Existing commands that call `scripts\desktop-regression\run.ps1` still forward
to the canonical runner.
